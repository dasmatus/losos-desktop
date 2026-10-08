//! The installer's backend: what `derisk installer` on the ISO talks to.
//!
//! derisk draws the installer and joins networks itself; this answers its
//! requests in derisk's JSON-lines protocol (derisk's `src/install.rs`), one
//! object per line, requests on stdin and events on stdout:
//!
//! - it says `hello` first, with the system's name, the channel it installs
//!   from and, when a release disk is mounted, `release`, its directory, which
//!   is installed from instead, so no network is needed; `hello` is said
//!   again before a `disks` answer whenever that changed;
//! - `disks` lists the disks [`crate::disks`] offers, the ISO's own and the
//!   release disk left out;
//! - `install` runs [`crate::install`] on the disk named, reporting `steps`,
//!   then a `step` and `line`s as it goes, and ends with `installed` or
//!   `failed`;
//! - `reboot` and `power_off` ask systemd.
//!
//! Nothing here decides what lands on the disk, any more than the text
//! interface this replaced did: repart and sysupdate do, from definitions the
//! OS generated.
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;

use serde_json::{Value, json};

use crate::disks::{self, Disk};
use crate::install::{self, Plan, Step};

/// What the backend was started with.
#[derive(Debug, Clone)]
pub struct Config {
    pub name: String,
    pub repart_definitions: PathBuf,
    pub sysupdate_templates: PathBuf,
    pub esp: PathBuf,
    /// Where the ISO is mounted, so its disk is never offered.
    pub medium: String,
    pub work: PathBuf,
    pub source: Option<String>,
    /// Where a release disk is mounted when one is attached
    /// ([`install::has_release`]).
    pub local_source: Option<PathBuf>,
    /// [`Plan::bios_boot`].
    pub bios_boot: Option<PathBuf>,
}

/// The release disk's directory, when one holding a release is mounted.
pub fn release(config: &Config) -> Option<&Path> {
    config
        .local_source
        .as_deref()
        .filter(|dir| install::has_release(dir))
}

pub fn hello(config: &Config) -> Value {
    let mut hello = json!({"event": "hello", "name": config.name, "source": config.source});
    if let Some(dir) = release(config) {
        hello["release"] = json!(dir.to_string_lossy());
    }
    hello
}

pub fn disks_event(disks: &[Disk]) -> Value {
    let disks: Vec<Value> = disks
        .iter()
        .map(|d| {
            json!({
                "path": d.path.to_string_lossy(),
                "name": d.name,
                "model": d.model,
                "size": d.size,
                "removable": d.removable,
            })
        })
        .collect();
    json!({"event": "disks", "disks": disks})
}

pub fn steps_event() -> Value {
    let labels: Vec<&str> = Step::ALL.iter().map(|s| s.title()).collect();
    json!({"event": "steps", "labels": labels})
}

/// A `step` event, with how far a download is when `percent` has a
/// reading from sysupdate's output.
pub fn step_event(step: Step, fraction: Option<f32>) -> Value {
    let index = Step::ALL.iter().position(|s| *s == step).unwrap_or(0);
    json!({
        "event": "step",
        "index": index,
        "count": Step::ALL.len(),
        "label": step.title(),
        "fraction": fraction,
    })
}

/// The last percentage in a line of systemd-pull's or sysupdate's output,
/// as a fraction: "Got 45% of https://…" is 0.45. Each file sysupdate
/// fetches counts from 0 to 100 again; the page shows the current one.
pub fn percent(line: &str) -> Option<f32> {
    let end = line.rfind('%')?;
    let digits: String = line[..end]
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let value: f32 = digits.parse().ok()?;
    (value <= 100.0).then_some(value / 100.0)
}

/// One request, parsed.
#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    Disks,
    Install(String),
    Reboot,
    PowerOff,
}

pub fn parse_request(line: &str) -> Result<Request, String> {
    let value: Value = serde_json::from_str(line.trim()).map_err(|e| format!("not JSON: {e}"))?;
    match value["method"].as_str() {
        Some("disks") => Ok(Request::Disks),
        Some("install") => value["disk"]
            .as_str()
            .filter(|d| d.starts_with("/dev/"))
            .map(|d| Request::Install(d.to_owned()))
            .ok_or_else(|| "install needs a disk under /dev".to_owned()),
        Some("reboot") => Ok(Request::Reboot),
        Some("power_off") => Ok(Request::PowerOff),
        other => Err(format!("unknown method {other:?}")),
    }
}

fn failed(message: impl Into<String>) -> Value {
    json!({"event": "failed", "message": message.into()})
}

/// The disks that can be installed onto, the medium's own left out, and the
/// release disk's: it is being read from, so it is no more a place to install
/// onto than the medium is.
///
/// sysfs and mountinfo are read before this returns; only the release disk's
/// filtering is left to the caller's walk.
fn current_disks(config: &Config) -> impl Iterator<Item = Disk> + use<> {
    let sys = Path::new("/sys");
    let info = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    let medium = disks::backing_disk(&info, sys, &config.medium);
    let release = config
        .local_source
        .as_deref()
        .and_then(|dir| disks::backing_disk(&info, sys, &dir.to_string_lossy()));
    disks::list(sys, medium.as_deref())
        .into_iter()
        .filter(move |disk| Some(&disk.name) != release.as_ref())
}

/// Serves requests from `input` until it closes, writing events to `output`.
pub fn run(config: &Config, input: impl BufRead, output: impl Write + Send + 'static) {
    let (events, outbox) = mpsc::channel::<Value>();
    // One writer, so an install's events and the answers to other requests
    // never interleave inside a line.
    let writer = thread::spawn(move || {
        let mut output = output;
        for event in outbox {
            if writeln!(output, "{event}")
                .and_then(|()| output.flush())
                .is_err()
            {
                return;
            }
        }
    });
    let mut said = hello(config);
    let _ = events.send(said.clone());

    let mut installing: Option<thread::JoinHandle<()>> = None;
    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let request = match parse_request(&line) {
            Ok(request) => request,
            Err(e) => {
                let _ = events.send(failed(e));
                continue;
            }
        };
        let busy = installing.as_ref().is_some_and(|t| !t.is_finished());
        match request {
            Request::Disks => {
                // udev mounts a release disk whenever it is plugged in, so
                // derisk hears of one the next time it asks for the disks.
                let now = hello(config);
                if now != said {
                    let _ = events.send(now.clone());
                    said = now;
                }
                let _ = events.send(disks_event(&current_disks(config).collect::<Vec<_>>()));
            }
            Request::Install(_) if busy => {
                let _ = events.send(failed("An install is already running."));
            }
            Request::Install(disk) => {
                // Only a disk this backend offered, which the person picked
                // from the list: never the medium, never a partition.
                if !current_disks(config).any(|d| d.path == Path::new(&disk)) {
                    let _ = events.send(failed(format!(
                        "{disk} is not a disk that can be installed onto."
                    )));
                    continue;
                }
                let plan = Plan {
                    disk: PathBuf::from(disk),
                    repart_definitions: config.repart_definitions.clone(),
                    sysupdate_templates: config.sysupdate_templates.clone(),
                    esp: config.esp.clone(),
                    work: config.work.clone(),
                    source: config.source.clone(),
                    local: config.local_source.clone(),
                    bios_boot: config.bios_boot.clone(),
                };
                let events = events.clone();
                installing = Some(thread::spawn(move || install_reporting(&plan, &events)));
            }
            Request::Reboot | Request::PowerOff if busy => {
                let _ = events.send(failed("An install is still running."));
            }
            Request::Reboot => power("reboot", &events),
            Request::PowerOff => power("poweroff", &events),
        }
    }
    if let Some(t) = installing {
        let _ = t.join();
    }
    drop(events);
    let _ = writer.join();
}

/// Runs the install and turns its events into the protocol's.
fn install_reporting(plan: &Plan, events: &mpsc::Sender<Value>) {
    let _ = events.send(steps_event());
    let (tx, rx) = mpsc::channel();
    let plan = plan.clone();
    let worker = thread::spawn(move || install::run(&plan, &tx));
    let mut step = Step::Partition;
    for event in rx {
        let out = match event {
            install::Event::Step(s) => {
                step = s;
                step_event(s, None)
            }
            install::Event::Line(text) => {
                if step == Step::Download
                    && let Some(fraction) = percent(&text)
                {
                    let _ = events.send(step_event(step, Some(fraction)));
                }
                json!({"event": "line", "text": text})
            }
            install::Event::Finished(Ok(())) => json!({"event": "installed"}),
            install::Event::Finished(Err(e)) => failed(e),
        };
        let _ = events.send(out);
    }
    let _ = worker.join();
}

fn power(verb: &str, events: &mpsc::Sender<Value>) {
    if let Err(e) = Command::new("systemctl").arg(verb).status() {
        let _ = events.send(failed(format!("systemctl {verb}: {e}")));
    }
}

/// Serves on this process's stdin and stdout.
pub fn serve_stdio(config: &Config) {
    run(config, io::stdin().lock(), io::stdout());
}
