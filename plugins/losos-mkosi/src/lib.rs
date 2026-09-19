//! A pm plugin that teaches pm about mkosi.
//!
//! `mkosi` is not in pm's fingerprint table -- it is listed by name in this
//! repository's `docs/pm-constraints.md` among the things an OS build reaches for
//! reflexively and pm refuses (C2) -- so before this plugin, a build file that
//! called it aborted before any step ran, with a diagnostic naming the command and
//! nothing else.
//!
//! That refusal is not pm being difficult. Its table is what makes `pm explain` a
//! real answer to "what does this build need", and a table that fell back to a
//! permissive default for anything it did not recognise would answer nothing. A
//! plugin is how a distribution extends the table without weakening it: pm only
//! ever asks about a command that matched **no** built-in fingerprint, so this can
//! name `mkosi` and can never reclassify `cargo` as needing no network.
//!
//! # Why mkosi gets no network
//!
//! mkosi normally installs a distribution's packages, which is a network build.
//! This one does not: the image layer hands mkosi a tree pm already built
//! (`BaseTrees=`, `Distribution=custom`), so there is nothing left to download. The
//! manifest's ceiling says so, which means pm would drop a network capability even
//! if a future edit here asked for one by mistake -- and network in a pm build file
//! is per *file*, not per step (C8), so one careless grant would put the whole image
//! layer's jail on the host network.
//!
//! An image build that reaches the network is an image build whose contents are not
//! the ones that were reviewed.
//!
//! # The symbols
//!
//! The other half of what a build file needs from mkosi is the handful of paths
//! mkosi and the Boot Loader Specification fix, which a step command would
//! otherwise spell out and get wrong. They are constants -- a plugin has no
//! filesystem and no environment, so there is nothing to compute them from -- and
//! `pm plugins` prints every one of them, so a reader can check what a `%{...}` in
//! a build file expanded to without reading this file.
//!
//! # What this can reach
//!
//! Nothing. The component imports one function, `log`, which returns nothing. No
//! filesystem, no clock, no network, no environment: pm hands over a command string
//! and takes an answer back.

wit_bindgen::generate!({ path: "../wit", world: "plugin" });

use pm::plugin::{
    host::{Level, log},
    types::{Capability, Hook, Permission, Symbol},
};

struct LososMkosi;

/// pm-fingerprints
///
/// The mkosi entry points a build step may name as its first word.
///
/// `mkosi-sandbox` is deliberately absent. mkosi execs it for itself from inside a
/// build, where pm's first-word resolution is never consulted; naming it here would
/// suggest a recipe could call it directly, and a recipe that did would be building
/// an image outside mkosi's own bookkeeping.
const ENTRY_POINTS: &[&str] = &["mkosi", "mkosi-initrd", "mkosi-addon"];

/// Paths mkosi and the Boot Loader Specification fix, and their values.
///
/// Each is a constant of the ecosystem rather than a choice this distribution made,
/// which is the whole test for whether something belongs here: a value a build file
/// is free to pick is a value that belongs in the build file.
const SYMBOLS: &[(&str, &str, &str)] = &[
    (
        "esp",
        "/efi",
        "where mkosi mounts the EFI system partition in an image it builds",
    ),
    (
        "xbootldr",
        "/boot",
        "where mkosi mounts the extended boot loader partition",
    ),
    (
        "uki-dir",
        "/efi/EFI/Linux",
        "the Boot Loader Specification type 2 directory, where UKIs go",
    ),
    (
        "loader-dir",
        "/efi/EFI/systemd",
        "where systemd-boot is installed on the ESP",
    ),
    (
        "config",
        "mkosi.conf",
        "the file name mkosi reads its configuration from",
    ),
];

/// The program name of a command, with any leading path removed.
///
/// pm allows an optional leading path in its own patterns (`/bin/sh`,
/// `./configure`), so a plugin that only matched a bare name would behave
/// differently from the built-in table for no reason a user could predict.
fn program(command: &str) -> Option<&str> {
    let first = command.split_whitespace().next()?;
    Some(first.rsplit('/').next().unwrap_or(first))
}

impl Guest for LososMkosi {
    fn describe() -> Manifest {
        Manifest {
            name: "losos-mkosi".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            summary: "Classifies mkosi, and names the paths it fixes".into(),
            // classify-command only. mkosi's configuration is not a source file in
            // the sense scan-source means, and claiming that hook would mean pm
            // handing this every file in every tree for nothing.
            hooks: vec![Hook::ClassifyCommand],
            grants_at_most: vec![
                Capability::Toolchain,
                Capability::Coreutils,
                Capability::Archive,
            ],
            source_extensions: vec![],
            symbols: SYMBOLS
                .iter()
                .map(|(name, value, summary)| Symbol {
                    name: (*name).into(),
                    value: (*value).into(),
                    summary: (*summary).into(),
                })
                .collect(),
        }
    }

    fn classify_command(command: String) -> Option<Verdict> {
        let program = program(&command)?;

        if !ENTRY_POINTS.contains(&program) {
            // No verdict rather than a guess. pm's answer to "nothing matched" is a
            // diagnostic naming the command, which is a better outcome than a jail
            // sized by a plugin that was not sure either.
            return None;
        }

        log(
            Level::Debug,
            &format!("classified `{program}` as an mkosi entry point"),
        );

        Some(Verdict {
            // Recorded as `losos-mkosi:mkosi`, so `pm explain` can never confuse a
            // plugin's answer for a built-in one.
            fingerprint: program.replace('.', "-"),
            capabilities: vec![
                // It drives mkfs, systemd-repart and ukify.
                Capability::Toolchain,
                // It copies and links trees for most of its run.
                Capability::Coreutils,
                // tar, cpio, xz and zstd, depending on the output format.
                Capability::Archive,
            ],
        })
    }

    /// Not implemented, but the component model has no optional export, so it exists
    /// and returns the answer that changes nothing.
    fn scan_source(_file: SourceFile) -> Vec<Grant> {
        Vec::new()
    }
}

export!(LososMkosi);

#[allow(dead_code)]
fn _permission_is_used_by_the_world(_p: Permission) {}
