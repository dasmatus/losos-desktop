//! The install, once there is room for it: the same steps whether the disk
//! is the running Windows machine's (windows.rs) or an image file
//! (image.rs), with [`Target`] standing for what differs.
use std::path::{Path, PathBuf};

use miette::{Result, WrapErr};

use crate::disk::{self, Device};
use crate::fat;
use crate::layout::Plan;
use crate::release::{Arch, Release};

/// Where GRUB goes on the ESP, beside Windows' own boot manager rather than
/// over the removable-media path, which is Windows' fallback. grub.nix puts
/// the same file at the same path on an ESP of LosOS's own.
pub fn loader_path(arch: &Arch) -> String {
    format!("EFI/losos/grub{}.efi", arch.efi)
}

/// GRUB's boot counter (grub.cfg), beside it.
pub const GRUBENV: &str = "EFI/losos/grubenv";

/// The ESP's record of the files this program put there, so `--uninstall`
/// removes those and nothing it found already there.
pub const MARKER: &str = "EFI/losos/losos-windows-installer.txt";

pub struct EspFile<'a> {
    pub path: String,
    pub bytes: &'a [u8],
    /// Whether a file already there is replaced or left as it is.
    pub replace: bool,
}

/// What differs between installing onto Windows' disk and onto an image.
pub trait Target {
    fn device(&mut self) -> &mut dyn Device;
    /// Adds the plan's partitions to the partition table.
    fn add_partitions(&mut self, plan: &Plan) -> Result<()>;
    /// Writes files onto the ESP and returns the paths that were not there
    /// before.
    fn put_esp(&mut self, files: &[EspFile]) -> Result<Vec<String>>;
    /// Registers `loader` on the ESP with the firmware, first in the order.
    fn add_boot_entry(&mut self, description: &str, loader: &str) -> Result<()>;
    /// Takes back whatever the calls above did, as far as it can.
    fn undo(&mut self) -> Result<()>;
}

/// The release's files, checked and on local disk.
pub struct Files {
    pub uki: PathBuf,
    pub usr: PathBuf,
    pub verity: PathBuf,
}

pub struct Boot<'a> {
    /// GRUB, with its menu and theme inside.
    pub loader: &'a [u8],
    /// An empty grubenv.
    pub grubenv: &'a [u8],
}

/// Steps as the person sees them.
pub trait Report {
    fn step(&mut self, title: &str);
    fn progress(&mut self, done: u64, total: u64);
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    target: &mut dyn Target,
    plan: &Plan,
    release: &Release,
    files: &Files,
    arch: &Arch,
    id: &str,
    boot: &Boot,
    report: &mut dyn Report,
) -> Result<()> {
    report.step("Adding LosOS's partitions");
    target.add_partitions(plan)?;

    for (title, part, path) in [
        (
            "Writing the system's hash tree",
            &plan.verity,
            &files.verity,
        ),
        ("Writing the system", &plan.usr, &files.usr),
    ] {
        report.step(title);
        disk::write_xz(
            target.device(),
            part.start,
            part.size,
            path,
            |done, total| report.progress(done, total),
        )?;
    }

    report.step("Writing the boot partition");
    let uki = format!("EFI/Linux/{}", release.uki_name(id));
    fat::format(
        target.device(),
        plan.boot.start,
        plan.boot.size,
        *b"LOSOS BOOT ",
        &[(uki.as_str(), files.uki.as_path())],
    )?;

    report.step("Adding GRUB to the EFI system partition");
    let loader = loader_path(arch);
    let mut created = target.put_esp(&[
        EspFile {
            path: loader.clone(),
            bytes: boot.loader,
            replace: true,
        },
        // Empty, so no trial an earlier install left goes on.
        EspFile {
            path: GRUBENV.into(),
            bytes: boot.grubenv,
            replace: true,
        },
    ])?;
    created.push(MARKER.to_owned());
    let marker = created.join("\n") + "\n";
    target
        .put_esp(&[EspFile {
            path: MARKER.into(),
            bytes: marker.as_bytes(),
            replace: true,
        }])
        .wrap_err("recording what was added to the ESP")?;

    report.step("Adding LosOS to the firmware's boot menu");
    target.add_boot_entry("LosOS", &format!("\\{}", loader.replace('/', "\\")))?;
    Ok(())
}

/// The local copies of a release's three files, each checked against its
/// line in the signed SHA256SUMS before it is used.
pub fn fetch_files(
    source: &crate::release::Source,
    release: &Release,
    cache: &Path,
    report: &mut dyn Report,
) -> Result<Files> {
    let mut get = |file: &crate::release::File| -> Result<PathBuf> {
        report.step(&format!("Getting {}", file.name));
        let path = source.fetch(&file.name, cache)?;
        let total = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let hash = crate::release::sha256_file(&path, |done| report.progress(done, total))?;
        if !hash.eq_ignore_ascii_case(&file.sha256) {
            // A bad download is not kept to be resumed from.
            if let crate::release::Source::Channel(_) = source {
                let _ = std::fs::remove_file(&path);
            }
            miette::bail!("{} does not match the release's SHA256SUMS", file.name);
        }
        Ok(path)
    };
    Ok(Files {
        uki: get(&release.uki)?,
        verity: get(&release.verity.file)?,
        usr: get(&release.usr.file)?,
    })
}
