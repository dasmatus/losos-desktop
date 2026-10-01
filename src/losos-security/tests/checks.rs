//! The checks, against fixture trees.
//!
//! Every check reads a path, and `--root` makes that path relative, so a
//! directory of files is a machine. That is the whole reason the rooting exists:
//! without it these could only be tested on hardware that happened to be
//! configured the way the assertion expects, which means they would not be
//! tested.
//!
//! Each check is asserted in BOTH directions. A security check that only ever
//! sees the passing case is the dangerous kind: it will report a pass on a
//! machine where the file it reads has moved, and nothing will say otherwise.

use std::fs;
use std::path::{Path, PathBuf};

use losos_security::attr::AttrResult;
use losos_security::checks::{self, Context};

fn write(root: &Path, relative: &str, contents: &[u8]) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn fixture(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("losos-security-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// A machine with everything on.
fn hardened(name: &str) -> PathBuf {
    let root = fixture(name);
    // Secure Boot: four attribute bytes then the value.
    write(
        &root,
        "sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c",
        &[0x06, 0x00, 0x00, 0x00, 0x01],
    );
    write(&root, "sys/class/tpm/tpm0/tpm_version_major", b"2\n");
    write(&root, "sys/kernel/security/tpm0/binary_bios_measurements", b"\x00");
    write(&root, "sys/block/dm-0/dm/uuid", b"CRYPT-LUKS2-deadbeef-root\n");
    write(&root, "sys/block/dm-0/dm/name", b"root\n");
    write(&root, "sys/block/dm-1/dm/uuid", b"CRYPT-VERITY-cafe-usr\n");
    write(&root, "sys/kernel/security/lockdown", b"none [integrity] confidentiality\n");
    write(&root, "sys/module/module/parameters/sig_enforce", b"Y\n");
    write(&root, "sys/class/iommu/dmar0/.keep", b"");
    write(&root, "proc/sys/kernel/kptr_restrict", b"2\n");
    write(&root, "proc/sys/kernel/dmesg_restrict", b"1\n");
    write(&root, "proc/sys/kernel/unprivileged_bpf_disabled", b"2\n");
    write(&root, "proc/sys/net/core/bpf_jit_harden", b"2\n");
    root
}

fn find<'a>(attrs: &'a [losos_security::attr::Attr], id: &str) -> &'a losos_security::attr::Attr {
    attrs
        .iter()
        .find(|a| a.appstream_id.ends_with(id))
        .unwrap_or_else(|| panic!("no check produced {id}"))
}

#[test]
fn a_hardened_machine_passes_everything() {
    let root = hardened("hardened");
    let attrs = checks::run(&Context::new(&root));

    for a in &attrs {
        assert!(a.success, "{} failed: {}", a.summary, a.evidence);
    }
    // Weakest link across levels 1..4; every check passed, so the highest
    // level that has checks is reached.
    assert_eq!(checks::overall_level(&attrs), 3);
}

#[test]
fn an_empty_machine_fails_everything() {
    // Not a machine with things turned off -- a machine where none of the
    // files exist at all, which is what a moved sysfs path looks like. Every
    // check must report a failure rather than a pass by absence.
    let root = fixture("empty");
    let attrs = checks::run(&Context::new(&root));

    for a in &attrs {
        assert!(!a.success, "{} passed on an empty tree: {}", a.summary, a.evidence);
    }
    assert_eq!(checks::overall_level(&attrs), 0);
}

#[test]
fn secure_boot_off_is_not_secure_boot_missing() {
    let root = hardened("sb-off");
    write(
        &root,
        "sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c",
        &[0x06, 0x00, 0x00, 0x00, 0x00],
    );
    let attrs = checks::run(&Context::new(&root));
    let sb = find(&attrs, "SecureBoot");

    assert!(!sb.success);
    // The distinction matters to whoever reads the report: "off" is a setting
    // they can change, "absent" means the machine did not boot via UEFI.
    assert_eq!(sb.result, AttrResult::NotEnabled);
}

#[test]
fn secure_boot_value_is_read_as_a_byte_not_as_text() {
    // The efivar is binary. A byte 0x00 is not the character '0', and reading
    // the file as a string would make a disabled machine look enabled.
    let root = hardened("sb-binary");
    write(
        &root,
        "sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c",
        &[0x06, 0x00, 0x00, 0x00, b'1'],
    );
    let attrs = checks::run(&Context::new(&root));
    // b'1' is 0x31, not 1, so this must NOT be read as enabled.
    assert!(!find(&attrs, "SecureBoot").success);
}

#[test]
fn a_tpm_1_2_is_not_a_tpm_2_0() {
    let root = hardened("tpm12");
    write(&root, "sys/class/tpm/tpm0/tpm_version_major", b"1\n");
    let attrs = checks::run(&Context::new(&root));
    let tpm = find(&attrs, "Tpm20");

    assert!(!tpm.success);
    assert_eq!(tpm.result, AttrResult::NotSupported);
}

#[test]
fn lockdown_reads_the_bracketed_mode() {
    // The file lists every mode and brackets the active one. Matching on
    // "integrity" anywhere in the text would pass on a machine in `none`.
    let root = hardened("lockdown-none");
    write(&root, "sys/kernel/security/lockdown", b"[none] integrity confidentiality\n");
    let attrs = checks::run(&Context::new(&root));
    let lockdown = find(&attrs, "KernelLockdown");

    assert!(!lockdown.success);
    assert_eq!(lockdown.result, AttrResult::NotLocked);
}

#[test]
fn a_plain_disk_is_not_reported_as_encrypted() {
    let root = hardened("plain-disk");
    fs::remove_dir_all(root.join("sys/block/dm-0")).unwrap();
    write(&root, "sys/block/sda/size", b"1024\n");
    let attrs = checks::run(&Context::new(&root));
    let crypt = find(&attrs, "DiskEncryption");

    assert!(!crypt.success);
    assert_eq!(crypt.result, AttrResult::NotEncrypted);
}

#[test]
fn a_sysctl_below_its_floor_fails() {
    let root = hardened("sysctl-low");
    write(&root, "proc/sys/kernel/kptr_restrict", b"0\n");
    let attrs = checks::run(&Context::new(&root));

    assert!(!find(&attrs, "KptrRestrict").success);
    // Its neighbours are unaffected: a shared helper that failed them all
    // together would hide which knob is actually wrong.
    assert!(find(&attrs, "DmesgRestrict").success);
}

#[test]
fn the_overall_level_is_the_weakest_link_not_an_average() {
    // Nine passes and one level-1 failure is a level-0 machine. Averaging that
    // away is how a report becomes decoration.
    let root = hardened("weakest-link");
    write(&root, "sys/class/tpm/tpm0/tpm_version_major", b"1\n");
    let attrs = checks::run(&Context::new(&root));

    let passed = attrs.iter().filter(|a| a.success).count();
    assert!(passed >= 9, "expected most checks to still pass, got {passed}");
    assert_eq!(checks::overall_level(&attrs), 0);
}

#[test]
fn every_check_reports_where_it_looked() {
    // A verdict without evidence cannot be argued with, and a security report
    // nobody can correct is a security report nobody should trust.
    let root = hardened("evidence");
    for a in checks::run(&Context::new(&root)) {
        assert!(!a.evidence.is_empty(), "{} has no evidence", a.summary);
        assert!(
            a.evidence.contains('/'),
            "{} does not name a path: {}",
            a.summary,
            a.evidence
        );
    }
}
