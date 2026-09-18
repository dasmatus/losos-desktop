//! The checks.
//!
//! fwupd's Host Security ID answers "is this machine's *firmware* trustworthy".
//! That is a real question and not the one a desktop user is asking, which is
//! closer to "is this machine set up the way a careful person would set it up".
//! lynis answers that for servers, in a shell script, with a report nobody
//! reads twice.
//!
//! So these are the OS-level half: TPM, disk encryption, kernel hardening,
//! boot integrity and unit sandboxing. They are emitted in fwupd's attribute
//! shape (see `attr.rs`) so GNOME renders them in the same list as the firmware
//! ones, which is the point -- a security report split across two applications
//! is two reports.
//!
//! # Every check names where it looked
//!
//! A check that reports a verdict without its evidence cannot be argued with,
//! and a security report nobody can argue with is a security report nobody can
//! correct. Each `Attr` carries the path it read and what it found, and the
//! panel shows it in the expanded row.
//!
//! # Every check is rooted
//!
//! Nothing here opens an absolute path directly: they all go through
//! [`Context::path`], which prefixes a configurable root. That is what makes
//! the suite testable -- `tests/` builds a directory of fixture sysfs files and
//! asserts the verdicts -- and it costs one function call.

use std::fs;
use std::path::{Path, PathBuf};

use crate::attr::{Attr, AttrResult};

pub struct Context {
    root: PathBuf,
}

impl Context {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Resolve an absolute system path against this context's root.
    fn path(&self, absolute: &str) -> PathBuf {
        self.root.join(absolute.trim_start_matches('/'))
    }

    fn read(&self, absolute: &str) -> Option<String> {
        fs::read_to_string(self.path(absolute))
            .ok()
            .map(|text| text.trim().to_string())
    }

    fn exists(&self, absolute: &str) -> bool {
        self.path(absolute).exists()
    }

    /// Entries of a directory, sorted, or empty when it does not exist.
    fn entries(&self, absolute: &str) -> Vec<String> {
        let mut found: Vec<String> = fs::read_dir(self.path(absolute))
            .map(|dir| {
                dir.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        found.sort();
        found
    }
}

/// Build one attribute. Kept as a function rather than a constructor so each
/// check below reads as a sentence.
#[allow(clippy::too_many_arguments)]
fn attr(
    id: &str,
    summary: &str,
    description: &str,
    result: AttrResult,
    hsi_level: u32,
    success: bool,
    evidence: String,
) -> Attr {
    Attr {
        appstream_id: format!("io.losos.SecurityAttr.{id}"),
        summary: summary.to_string(),
        description: description.to_string(),
        result,
        hsi_level,
        success,
        evidence,
    }
}

// ---------------------------------------------------------------- TPM ----

/// A TPM 2.0 has to exist before anything can be sealed to it.
///
/// 1.2 is deliberately reported as not-supported rather than as a pass: it
/// cannot do the PCR policies systemd-cryptenroll binds a key with, so
/// treating it as "a TPM is present" would be reassuring and wrong.
fn tpm_present(ctx: &Context) -> Attr {
    let version = ctx.read("/sys/class/tpm/tpm0/tpm_version_major");
    let (result, success, evidence) = match version.as_deref() {
        Some("2") => (
            AttrResult::Found,
            true,
            "/sys/class/tpm/tpm0/tpm_version_major: 2".to_string(),
        ),
        Some(other) => (
            AttrResult::NotSupported,
            false,
            format!("/sys/class/tpm/tpm0/tpm_version_major: {other} (2.0 required)"),
        ),
        None => (
            AttrResult::NotFound,
            false,
            "/sys/class/tpm/tpm0: absent".to_string(),
        ),
    };
    attr(
        "Tpm20",
        "TPM 2.0 device",
        "A TPM 2.0 lets the system seal disk encryption keys to the exact \
         software that booted, so a disk removed from this machine cannot be \
         unlocked elsewhere.",
        result,
        1,
        success,
        evidence,
    )
}

/// Measured boot: the firmware and boot chain recorded themselves into PCRs.
///
/// The event log's existence is the signal. Without it the PCR values exist but
/// nothing can say what produced them, which makes a policy bound to them
/// unauditable.
fn measured_boot(ctx: &Context) -> Attr {
    let log = "/sys/kernel/security/tpm0/binary_bios_measurements";
    let present = ctx.exists(log);
    attr(
        "MeasuredBoot",
        "Measured boot",
        "Each stage of the boot recorded a measurement of the next into the \
         TPM. This is what makes it possible to tell whether the system that \
         booted is the system that was expected.",
        if present { AttrResult::Enabled } else { AttrResult::NotEnabled },
        2,
        present,
        format!("{log}: {}", if present { "present" } else { "absent" }),
    )
}

// --------------------------------------------------------- encryption ----

/// Whether any block device is a LUKS mapping.
///
/// device-mapper writes the mapping's UUID as `CRYPT-LUKS2-<uuid>-<name>`, so
/// the prefix is the check. Reading it from sysfs rather than shelling out to
/// cryptsetup keeps this working in an initrd and inside a test fixture.
fn disk_encryption(ctx: &Context) -> Attr {
    let mut encrypted = Vec::new();
    for device in ctx.entries("/sys/block") {
        let uuid = ctx.read(&format!("/sys/block/{device}/dm/uuid"));
        if let Some(uuid) = uuid {
            if uuid.starts_with("CRYPT-LUKS") {
                let name = ctx
                    .read(&format!("/sys/block/{device}/dm/name"))
                    .unwrap_or_else(|| device.clone());
                encrypted.push(name);
            }
        }
    }

    let found = !encrypted.is_empty();
    attr(
        "DiskEncryption",
        "Disk encryption",
        "Personal files are stored on an encrypted volume, so removing the \
         drive does not reveal them.",
        if found { AttrResult::Encrypted } else { AttrResult::NotEncrypted },
        1,
        found,
        if found {
            format!("/sys/block/*/dm/uuid: LUKS mapping(s) {}", encrypted.join(", "))
        } else {
            "/sys/block/*/dm/uuid: no CRYPT-LUKS mapping".to_string()
        },
    )
}

/// dm-verity on /usr: the OS image is verified block by block as it is read.
fn verity_usr(ctx: &Context) -> Attr {
    let mut verity = Vec::new();
    for device in ctx.entries("/sys/block") {
        if let Some(uuid) = ctx.read(&format!("/sys/block/{device}/dm/uuid")) {
            if uuid.starts_with("CRYPT-VERITY") {
                verity.push(device);
            }
        }
    }

    let found = !verity.is_empty();
    attr(
        "VerityUsr",
        "Verified system image",
        "The operating system's files are checked against a signed hash tree \
         every time they are read, so a modified system file cannot go \
         unnoticed.",
        if found { AttrResult::Valid } else { AttrResult::NotValid },
        3,
        found,
        if found {
            format!("/sys/block/*/dm/uuid: verity target(s) {}", verity.join(", "))
        } else {
            "/sys/block/*/dm/uuid: no CRYPT-VERITY mapping".to_string()
        },
    )
}

// ------------------------------------------------------------- kernel ----

/// UEFI Secure Boot, read from the efivar rather than from a bootloader claim.
///
/// The variable's first four bytes are EFI attributes; the fifth is the value.
/// Reading it as text and looking for a `1` would be wrong -- it is binary, and
/// on a machine with Secure Boot off the byte is 0x00, which is not the
/// character '0'.
fn secure_boot(ctx: &Context) -> Attr {
    const VAR: &str =
        "/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c";
    let raw = fs::read(ctx.path(VAR)).ok();
    let (result, success, evidence) = match raw.as_deref() {
        Some(bytes) if bytes.len() >= 5 && bytes[4] == 1 => (
            AttrResult::Enabled,
            true,
            format!("{VAR}: byte 4 = 1"),
        ),
        Some(bytes) if bytes.len() >= 5 => (
            AttrResult::NotEnabled,
            false,
            format!("{VAR}: byte 4 = {}", bytes[4]),
        ),
        Some(_) => (
            AttrResult::Unknown,
            false,
            format!("{VAR}: shorter than 5 bytes"),
        ),
        None => (
            AttrResult::NotFound,
            false,
            format!("{VAR}: absent (not booted via UEFI?)"),
        ),
    };
    attr(
        "SecureBoot",
        "Secure Boot",
        "The firmware refused to run any boot code that was not signed by a \
         key it trusts.",
        result,
        1,
        success,
        evidence,
    )
}

/// Kernel lockdown, which is what stops root from reading kernel memory.
///
/// The file reports every mode with the active one in brackets, e.g.
/// `none [integrity] confidentiality`, so the bracket is the reading.
fn kernel_lockdown(ctx: &Context) -> Attr {
    let raw = ctx.read("/sys/kernel/security/lockdown");
    let active = raw.as_deref().and_then(|text| {
        text.split_whitespace()
            .find(|word| word.starts_with('['))
            .map(|word| word.trim_matches(['[', ']']).to_string())
    });

    let (result, success) = match active.as_deref() {
        Some("integrity") | Some("confidentiality") => (AttrResult::Locked, true),
        Some(_) => (AttrResult::NotLocked, false),
        None => (AttrResult::NotFound, false),
    };

    attr(
        "KernelLockdown",
        "Kernel lockdown",
        "Even an administrator cannot read or modify the running kernel's \
         memory, which is what keeps a compromised account from becoming a \
         compromised kernel.",
        result,
        3,
        success,
        match raw {
            Some(text) => format!("/sys/kernel/security/lockdown: {text}"),
            None => "/sys/kernel/security/lockdown: absent".to_string(),
        },
    )
}

/// Module signature enforcement: the kernel refuses unsigned modules.
fn module_signing(ctx: &Context) -> Attr {
    let raw = ctx.read("/sys/module/module/parameters/sig_enforce");
    let enforced = raw.as_deref() == Some("Y");
    attr(
        "ModuleSigning",
        "Kernel module signatures",
        "The kernel loads only modules signed by a trusted key, so a driver \
         cannot be replaced without that key.",
        if enforced { AttrResult::Enabled } else { AttrResult::NotEnabled },
        3,
        enforced,
        format!(
            "/sys/module/module/parameters/sig_enforce: {}",
            raw.unwrap_or_else(|| "absent".to_string())
        ),
    )
}

/// An IOMMU, without which any DMA-capable device can read all of memory.
fn iommu(ctx: &Context) -> Attr {
    let groups = ctx.entries("/sys/class/iommu");
    let present = !groups.is_empty();
    attr(
        "Iommu",
        "Device memory protection",
        "Plugged-in devices are prevented from reading memory that does not \
         belong to them, which is the defence against a malicious peripheral.",
        if present { AttrResult::Enabled } else { AttrResult::NotEnabled },
        2,
        present,
        format!(
            "/sys/class/iommu: {}",
            if present { groups.join(", ") } else { "empty".to_string() }
        ),
    )
}

/// A sysctl that must be at or above a floor.
fn sysctl_at_least(
    ctx: &Context,
    id: &str,
    summary: &str,
    description: &str,
    knob: &str,
    minimum: i64,
    hsi_level: u32,
) -> Attr {
    let path = format!("/proc/sys/{}", knob.replace('.', "/"));
    let raw = ctx.read(&path);
    let value: Option<i64> = raw.as_deref().and_then(|t| t.parse().ok());
    let ok = value.map(|v| v >= minimum).unwrap_or(false);

    attr(
        id,
        summary,
        description,
        if ok { AttrResult::Enabled } else { AttrResult::NotEnabled },
        hsi_level,
        ok,
        match raw {
            Some(text) => format!("{path}: {text} (want >= {minimum})"),
            None => format!("{path}: absent"),
        },
    )
}

// ------------------------------------------------------------ the set ----

/// Run every check. Order is the order they are shown.
pub fn run(ctx: &Context) -> Vec<Attr> {
    vec![
        secure_boot(ctx),
        tpm_present(ctx),
        measured_boot(ctx),
        disk_encryption(ctx),
        verity_usr(ctx),
        kernel_lockdown(ctx),
        module_signing(ctx),
        iommu(ctx),
        sysctl_at_least(
            ctx,
            "KptrRestrict",
            "Kernel address exposure",
            "Kernel memory addresses are hidden from unprivileged programs, \
             which removes the easiest way to defeat address randomisation.",
            "kernel.kptr_restrict",
            1,
            2,
        ),
        sysctl_at_least(
            ctx,
            "DmesgRestrict",
            "Kernel log access",
            "The kernel log is readable only by administrators. It routinely \
             contains memory addresses and hardware details.",
            "kernel.dmesg_restrict",
            1,
            2,
        ),
        sysctl_at_least(
            ctx,
            "UnprivilegedBpf",
            "Unprivileged BPF",
            "Ordinary users cannot load BPF programs into the kernel. BPF is \
             powerful by design and has been a recurring source of privilege \
             escalation.",
            "kernel.unprivileged_bpf_disabled",
            1,
            3,
        ),
        sysctl_at_least(
            ctx,
            "BpfJitHarden",
            "BPF JIT hardening",
            "Kernel-generated machine code is hardened against being used as \
             an attack primitive.",
            "net.core.bpf_jit_harden",
            1,
            3,
        ),
    ]
}

/// The overall level, in fwupd's terms: the highest level at which every check
/// passed.
///
/// Deliberately the *weakest link* rather than a score or a percentage. A
/// machine with nine passes and one failure at level 1 is a machine with a
/// level 0 problem, and averaging that away is how a report becomes decoration.
pub fn overall_level(attrs: &[Attr]) -> u32 {
    let mut level = 0;
    for candidate in 1..=4 {
        let mut at_this_level = attrs.iter().filter(|a| a.hsi_level == candidate).peekable();

        // A level with no checks does NOT count as passed. `all()` on an empty
        // iterator is true, so without this a machine would be reported at the
        // highest level in the scale simply because nothing tests it -- the
        // most flattering possible answer, arrived at by testing nothing.
        if at_this_level.peek().is_none() {
            break;
        }

        if at_this_level.all(|a| a.success) {
            level = candidate;
        } else {
            break;
        }
    }
    level
}

pub fn default_root() -> &'static Path {
    Path::new("/")
}
