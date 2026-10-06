//! The install itself: systemd-repart lays out the disk, systemd-sysupdate
//! fills it.
//!
//! repart creates the ESP, with systemd-boot and loader.conf copied in, and
//! both slots' /usr partitions, empty and labelled `_empty`. That label is
//! what sysupdate looks for when it needs a partition to write a version
//! into, so the install that follows is an ordinary update: sysupdate reads
//! SHA256SUMS from the channel, downloads the newest UKI and both /usr halves,
//! writes the halves into one slot, relabels them with the version, and puts
//! the UKI on the ESP. Everything else -- root, /home, swap -- is made by the
//! installed system's first boot, as it is for an image written with dd.
//!
//! With a release disk attached, the same transfers read from it instead of
//! the channel: their sources become local files, and the files are checked
//! against the release's signed SHA256SUMS here first, since sysupdate only
//! verifies signatures on what it downloads. That is how an install works
//! where the channel cannot be reached, such as a VM whose network
//! re-signs TLS with a certificate authority the installer does not know.
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
    /// The channel's URL, as the transfers' sources name it.
    pub source: Option<String>,
    /// A directory holding a release (its SHA256SUMS, SHA256SUMS.gpg and the
    /// files they list), used in place of the channel when [`has_release`]
    /// says one is there.
    pub local: Option<PathBuf>,
}

/// Where the release signing key is, as sysupdate itself reads it.
const PUBRING: &str = "/etc/systemd/import-pubring.gpg";

/// Whether `dir` holds a release to install from.
pub fn has_release(dir: &Path) -> bool {
    dir.join("SHA256SUMS").is_file()
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

    /// What the installer's page says while this step runs.
    pub fn title(self) -> &'static str {
        match self {
            Step::Partition => "Partitioning the disk",
            Step::Mount => "Mounting the new boot partition",
            Step::Download => "Downloading and writing the system",
            Step::Finish => "Finishing",
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
    match (downloaded, unmounted) {
        (Ok(()), Ok(())) => {}
        (Err(download), Ok(())) => return Err(download),
        (Ok(()), Err(unmount)) => return Err(unmount),
        (Err(download), Err(unmount)) => {
            return Err(format!("{download}; cleanup also failed: {unmount}"));
        }
    }
    command("sync", &[], false, events)?;
    Ok(())
}

fn download(plan: &Plan, disk: &str, events: &Sender<Event>) -> Result<(), String> {
    let linux = plan.esp.join("EFI/Linux");
    fs::create_dir_all(&linux).map_err(|e| format!("{}: {e}", linux.display()))?;
    let transfers = plan.work.join("sysupdate.d");
    let local = match (&plan.source, &plan.local) {
        (Some(url), Some(dir)) if has_release(dir) => {
            let _ = events.send(Event::Line(format!(
                "Installing from the release in {} instead of {url}",
                dir.display()
            )));
            verify_release(dir, Path::new(PUBRING), &plan.work, events)?;
            Some(Local { url, dir })
        }
        _ => None,
    };
    render_transfers(&plan.sysupdate_templates, &transfers, disk, local)?;
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

/// A release directory standing in for the channel at `url`.
#[derive(Debug, Clone, Copy)]
pub struct Local<'a> {
    pub url: &'a str,
    pub dir: &'a Path,
}

/// Copies every transfer in `templates` into `out`, naming `disk` as the
/// partition target, and with `local`, reading from its directory rather
/// than from the channel.
pub fn render_transfers(
    templates: &Path,
    out: &Path,
    disk: &str,
    local: Option<Local>,
) -> Result<(), String> {
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
        let text = match local {
            Some(local) => localise(&text, local),
            None => text,
        };
        let target = out.join(template.file_name().unwrap_or_default());
        fs::write(&target, text.replace(TARGET, disk)).map_err(|e| fail(&target, e))?;
    }
    Ok(())
}

/// A transfer whose source is the channel, turned into one whose source is
/// the same files in a directory. Only the source's own two lines change:
/// the patterns stay, so sysupdate picks the newest version there exactly as
/// it would from the channel, and `Verify=` stays too, though sysupdate only
/// acts on it for downloads ([`verify_release`] stands in for it).
fn localise(text: &str, local: Local) -> String {
    let path = format!("Path={}", local.url);
    text.lines()
        .map(|line| match line.trim() {
            "Type=url-file" => "Type=regular-file".to_owned(),
            trimmed if trimmed == path => format!("Path={}", local.dir.display()),
            _ => line.to_owned(),
        })
        .map(|line| line + "\n")
        .collect()
}

/// Checks a release directory as sysupdate checks a download: SHA256SUMS
/// signed by the release key, and then every file beside it listed there
/// with the hash it has. A file it does not list is refused rather than
/// skipped, because sysupdate would install it all the same if its name
/// matched. Without a key on the system, as without one sysupdate verifies
/// nothing, only the hashes are checked.
pub fn verify_release(
    dir: &Path,
    pubring: &Path,
    work: &Path,
    events: &Sender<Event>,
) -> Result<(), String> {
    let sums = dir.join("SHA256SUMS");
    let sums_text = sums.to_string_lossy().into_owned();
    if pubring.exists() {
        let home = work.join("gnupg");
        fs::create_dir_all(&home).map_err(|e| format!("{}: {e}", home.display()))?;
        let signature = dir.join("SHA256SUMS.gpg").to_string_lossy().into_owned();
        // The options systemd-pull passes: only the given keyring, and any
        // key in it is trusted, since being in it is what trust means here.
        command(
            "gpg",
            &[
                "--homedir",
                &home.to_string_lossy(),
                "--no-options",
                "--no-default-keyring",
                "--no-auto-key-locate",
                "--no-auto-check-trustdb",
                "--batch",
                "--trust-model=always",
                &format!("--keyring={}", pubring.display()),
                "--verify",
                &signature,
                &sums_text,
            ],
            false,
            events,
        )?;
    } else {
        let _ = events.send(Event::Line(format!(
            "{} is missing, so the release's signature is not checked",
            pubring.display()
        )));
    }

    let listed = parse_sums(&fs::read_to_string(&sums).map_err(|e| format!("{sums_text}: {e}"))?);
    let mut names: Vec<String> = fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name != "SHA256SUMS" && name != "SHA256SUMS.gpg")
        .collect();
    names.sort();
    for name in names {
        let Some(expected) = listed.iter().find(|(_, n)| *n == name).map(|(h, _)| h) else {
            return Err(format!("{name} is not in the release's SHA256SUMS"));
        };
        let path = dir.join(&name).to_string_lossy().into_owned();
        let out = command("sha256sum", &[&path], true, events)?;
        let actual = out.split_whitespace().next().unwrap_or_default();
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(format!("{name} does not match the release's SHA256SUMS"));
        }
        let _ = events.send(Event::Line(format!("{name}: OK")));
    }
    Ok(())
}

/// The (hash, file name) pairs of a SHA256SUMS file, in either of
/// sha256sum's text (`hash  name`) or binary (`hash *name`) forms.
pub fn parse_sums(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let (hash, name) = line.split_once(' ')?;
            let name = name.strip_prefix([' ', '*']).unwrap_or(name);
            (hash.len() == 64 && !name.is_empty()).then(|| (hash.to_owned(), name.to_owned()))
        })
        .collect()
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

/// Runs `program`, sending each line it logs to the installer. With `capture`,
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
        // Plain text: the installer's details are not a terminal and show
        // escapes as-is.
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
