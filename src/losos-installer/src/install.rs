//! The install itself: systemd-repart lays out the disk, systemd-sysupdate
//! fills it.
//!
//! repart creates the ESP, with systemd-boot and loader.conf copied in, and
//! slot A's two /usr partitions, empty and labelled `_empty`. That label is
//! what sysupdate looks for when it needs a partition to write a version
//! into, so the install that follows is an ordinary update: sysupdate reads
//! SHA256SUMS from the channel, downloads the newest UKI and both /usr halves,
//! writes the halves into slot A, relabels them with the version, and puts the
//! UKI on the ESP. Everything else -- slot B, root, /home, swap -- is made by
//! the installed system's first boot, as it is for an image written with dd.
//!
//! sysupdate's partition targets name the disk they write to, which the OS
//! cannot know when it writes the transfers, so they say [`TARGET`] and are
//! rendered for the chosen disk here. The UKI's target is the ESP's mount
//! point, which the OS does know, because it passes the same path to this
//! program.
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Duration, Instant};

/// Stands for the target disk's device node in the transfer templates.
pub const TARGET: &str = "@TARGET@";

#[derive(Debug, Clone)]
pub struct Plan {
    pub disk: PathBuf,
    pub repart_definitions: PathBuf,
    pub sysupdate_templates: PathBuf,
    pub esp: PathBuf,
    pub work: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Partition,
    Mount,
    Download,
    Finish,
}

impl Step {
    pub const ALL: [Step; 4] = [Step::Partition, Step::Mount, Step::Download, Step::Finish];

    pub fn label(self) -> &'static str {
        match self {
            Step::Partition => "Partition the disk (systemd-repart)",
            Step::Mount => "Mount the new ESP",
            Step::Download => "Download and write slot A (systemd-sysupdate)",
            Step::Finish => "Unmount and flush",
        }
    }
}

#[derive(Debug)]
pub enum Event {
    Step(Step),
    Line(String),
    Finished(Result<(), String>),
}

/// Runs the whole install, reporting as it goes. Always ends with
/// [`Event::Finished`].
pub fn run(plan: &Plan, events: &Sender<Event>) {
    let result = install(plan, events);
    let _ = events.send(Event::Finished(result));
}

fn install(plan: &Plan, events: &Sender<Event>) -> Result<(), String> {
    let step = |s| {
        let _ = events.send(Event::Step(s));
    };

    step(Step::Partition);
    let disk = plan.disk.to_string_lossy().into_owned();
    let definitions = format!("--definitions={}", plan.repart_definitions.display());
    // --empty=force writes a fresh partition table whatever was there; the
    // person typed `erase` to get here.
    let table = command(
        "systemd-repart",
        &[
            "--dry-run=no",
            "--empty=force",
            &definitions,
            "--json=short",
            &disk,
        ],
        true,
        events,
    )?;
    let esp = esp_node(&table)?;

    step(Step::Mount);
    // The kernel has the new table; udev still has to make the node.
    command("udevadm", &["settle"], false, events)?;
    wait_for(&esp, Duration::from_secs(10))?;
    fs::create_dir_all(&plan.esp).map_err(|e| format!("{}: {e}", plan.esp.display()))?;
    let esp_text = esp.to_string_lossy().into_owned();
    let mount_point = plan.esp.to_string_lossy().into_owned();
    command(
        "mount",
        &["-t", "vfat", "-o", "umask=0077", &esp_text, &mount_point],
        false,
        events,
    )?;

    step(Step::Download);
    let downloaded = download(plan, &disk, events);
    if downloaded.is_ok() {
        step(Step::Finish);
    }
    // Unmount even when the download failed, so the next attempt can
    // repartition the disk.
    let unmounted = command("umount", &[&mount_point], false, events).map(|_| ());
    downloaded.and(unmounted)?;
    command("sync", &[], false, events)?;
    Ok(())
}

fn download(plan: &Plan, disk: &str, events: &Sender<Event>) -> Result<(), String> {
    let linux = plan.esp.join("EFI/Linux");
    fs::create_dir_all(&linux).map_err(|e| format!("{}: {e}", linux.display()))?;
    let transfers = plan.work.join("sysupdate.d");
    render_transfers(&plan.sysupdate_templates, &transfers, disk)?;
    let definitions = format!("--definitions={}", transfers.display());
    command(
        "systemd-sysupdate",
        &[&definitions, "update"],
        false,
        events,
    )
    .map(|_| ())
}

/// The ESP's device node, from repart's JSON summary of the table it wrote.
pub fn esp_node(json: &str) -> Result<PathBuf, String> {
    let table: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("systemd-repart's output: {e}"))?;
    table
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["type"] == "esp")
        .and_then(|p| p["node"].as_str())
        .map(PathBuf::from)
        .ok_or_else(|| "systemd-repart reported no ESP".to_owned())
}

/// Copies every transfer in `templates` into `out`, naming `disk` as the
/// partition target.
pub fn render_transfers(templates: &Path, out: &Path, disk: &str) -> Result<(), String> {
    let fail = |path: &Path, e: std::io::Error| format!("{}: {e}", path.display());
    if out.exists() {
        fs::remove_dir_all(out).map_err(|e| fail(out, e))?;
    }
    fs::create_dir_all(out).map_err(|e| fail(out, e))?;
    let mut entries: Vec<_> = fs::read_dir(templates)
        .map_err(|e| fail(templates, e))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "transfer"))
        .collect();
    entries.sort();
    if entries.is_empty() {
        return Err(format!("{}: no transfers", templates.display()));
    }
    for template in entries {
        let text = fs::read_to_string(&template).map_err(|e| fail(&template, e))?;
        let target = out.join(template.file_name().unwrap_or_default());
        fs::write(&target, text.replace(TARGET, disk)).map_err(|e| fail(&target, e))?;
    }
    Ok(())
}

fn wait_for(path: &Path, timeout: Duration) -> Result<(), String> {
    let start = Instant::now();
    while !path.exists() {
        if start.elapsed() > timeout {
            return Err(format!("{} did not appear", path.display()));
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

/// Runs `program`, sending each line it logs to the screen. With `capture`,
/// standard output is returned rather than shown.
fn command(
    program: &str,
    args: &[&str],
    capture: bool,
    events: &Sender<Event>,
) -> Result<String, String> {
    let _ = events.send(Event::Line(format!("$ {program} {}", args.join(" "))));
    let mut child = Command::new(program)
        .args(args)
        // Plain text: the log pane is not a terminal and shows escapes as-is.
        .env("SYSTEMD_COLORS", "0")
        .env("SYSTEMD_PAGER", "")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;

    let forward = |stream: Box<dyn Read + Send>| {
        let events = events.clone();
        thread::spawn(move || {
            // systemd-pull reports progress with carriage returns.
            for line in BufReader::new(stream).split(b'\n').map_while(Result::ok) {
                for part in String::from_utf8_lossy(&line).split('\r') {
                    if !part.trim().is_empty() {
                        let _ = events.send(Event::Line(part.trim_end().to_owned()));
                    }
                }
            }
        })
    };
    let stderr = forward(Box::new(child.stderr.take().expect("piped")));
    let mut captured = String::new();
    let stdout = child.stdout.take().expect("piped");
    let stdout_thread = if capture {
        let mut stdout = stdout;
        stdout
            .read_to_string(&mut captured)
            .map_err(|e| format!("{program}: {e}"))?;
        None
    } else {
        Some(forward(Box::new(stdout)))
    };

    let status = child.wait().map_err(|e| format!("{program}: {e}"))?;
    let _ = stderr.join();
    if let Some(t) = stdout_thread {
        let _ = t.join();
    }
    if status.success() {
        Ok(captured)
    } else {
        Err(format!("{program} failed ({status})"))
    }
}
