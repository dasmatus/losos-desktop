//! A pm plugin that teaches pm about Nix.
//!
//! This repository builds the same OS twice: the pm chain, and the NixOS flake
//! beside it (`docs/nixos.md`). pm stays the system manager on both, which means
//! pm is also what a user reaches for to drive the flake -- a build file whose
//! steps are `nix flake check` and `nix build .#image`. pm's fingerprint table
//! knows neither word, so before this plugin such a file was refused before any
//! step ran (C2), with a diagnostic naming the command and nothing else.
//!
//! # What gets a verdict
//!
//! The commands that *read or build* a flake: `nix build`, `nix eval`, `nix
//! flake check|show|metadata`, `nix path-info`, `nix log`, `nix derivation
//! show`, the read-only `nix store` subcommands, `nix hash` and `nix nar`, and
//! their older spellings `nix-build`, `nix-instantiate`, `nix-hash` and the
//! querying operations of `nix-store`.
//!
//! # What is refused, and why that is a feature
//!
//! A plugin cannot make pm refuse anything -- `None` only leaves pm's own "no
//! fingerprint matches" in place. So the refusals below are commands this plugin
//! *recognises* and deliberately declines to size a jail for, with a line in the
//! log saying why, rather than commands it has never heard of:
//!
//! * `nixos-rebuild`, `nixos-install`, `nix-env`, `nix profile`: they change a
//!   machine rather than build something. A build step has no business changing
//!   the machine it runs on, and this OS is not updated that way at all -- an
//!   update is a new `/usr` from systemd-sysupdate (`nixos/modules/update.nix`),
//!   and the image carries no Nix to run a switch with.
//! * `nix run|shell|develop|fmt|bundle|repl|env`, `nix-shell`: each runs a
//!   program the step does not name. A jail sized for `nix` is not a jail sized
//!   for whatever the flake's app turns out to be.
//! * `nix copy`, `nix-copy-closure`, `nix-store --serve|--import|--gc|...`: they
//!   write to or serve a store other than reading the build's own.
//! * `nix flake update|lock|prefetch`, `nix-prefetch-url`, `nix-channel`: they
//!   fetch something newer than what is pinned. A pin moves by hand in this tree
//!   (CLAUDE.md, "Keeping the pins current"), and `flake.lock` is one.
//!
//! # Network, and `--offline`
//!
//! Evaluating a flake can fetch its inputs and building one substitutes from
//! cache.nixos.org, so a verdict normally carries [`Capability::Network`], as
//! pm's own table gives it to `cargo`. The exception is a `nix` command that
//! says `--offline`: that one gets no network, so the jail holds the recipe to
//! what it claimed. `--offline` on its own only turns substituters off and trusts
//! whatever was already downloaded; it is pm's jail that turns "should not fetch"
//! into "cannot fetch". The older `nix-*` commands have no such switch and always
//! get the network. `nix hash` and `nix nar` never do: they read a file.
//!
//! Network in a pm build file is per *file*, not per step (C8), so one step that
//! gets it puts every step of that file on the host network. A build file meant
//! to stay offline has to say `--offline` on every step, and `pm explain` shows
//! whether it did: the fingerprint of an offline step ends in `-offline`.
//!
//! # What this can reach
//!
//! Nothing. The component imports one function, `log`, which returns nothing. pm
//! hands over a command string and takes an answer back.

wit_bindgen::generate!({ path: "../wit", world: "plugin" });

use pm::plugin::{
    host::{Level, log},
    types::{Capability, Hook, Permission, Symbol},
};

struct LososNix;

/// What this plugin concluded about one command.
#[derive(Debug, PartialEq)]
enum Decision {
    /// Recognised and classified.
    Classify { fingerprint: String, network: bool },
    /// Recognised, and deliberately given no verdict. The reason goes in the log.
    Refuse(&'static str),
    /// Not a command this plugin knows.
    Unknown,
}

/// `nix` subcommands that read or build and never reach the network: they read a
/// file or a NAR the step already has.
const LOCAL: &[&str] = &["hash", "nar"];

/// `nix` subcommands that read or build and may fetch while doing so.
const FETCHING: &[&str] = &["build", "eval", "path-info", "log", "why-depends"];

/// `nix` subcommands that group further subcommands, and the ones of those that
/// read or build.
const GROUPS: &[(&str, &[&str])] = &[
    ("flake", &["check", "show", "metadata", "info"]),
    ("derivation", &["show"]),
    (
        "store",
        &[
            "ls",
            "cat",
            "dump-path",
            "verify",
            "diff-closures",
            "path-from-hash-part",
        ],
    ),
];

/// Why a machine-changing command gets no verdict.
const CHANGES_THE_MACHINE: &str = "changes a machine rather than building something; \
     this OS updates by systemd-sysupdate, never by activating a new generation";

/// Why a command that runs something else gets no verdict.
const RUNS_SOMETHING_ELSE: &str =
    "runs a program the step does not name, and a jail sized for nix is not sized for that";

/// Why a command that writes to or serves another store gets no verdict.
const OTHER_STORE: &str = "writes to or serves a store other than the build's own";

/// Why a command that moves an input past its pin gets no verdict.
const UNPINNED: &str = "fetches something newer than what is pinned; a pin moves by hand";

/// `nix` subcommands recognised and refused.
const REFUSED: &[(&str, &str)] = &[
    ("profile", CHANGES_THE_MACHINE),
    ("upgrade-nix", CHANGES_THE_MACHINE),
    ("registry", CHANGES_THE_MACHINE),
    ("daemon", OTHER_STORE),
    ("copy", OTHER_STORE),
    ("run", RUNS_SOMETHING_ELSE),
    ("shell", RUNS_SOMETHING_ELSE),
    ("develop", RUNS_SOMETHING_ELSE),
    ("fmt", RUNS_SOMETHING_ELSE),
    ("bundle", RUNS_SOMETHING_ELSE),
    ("repl", RUNS_SOMETHING_ELSE),
    ("env", RUNS_SOMETHING_ELSE),
];

/// `nix flake` subcommands recognised and refused.
const REFUSED_FLAKE: &[&str] = &["update", "lock", "prefetch", "clone"];

/// Programs other than `nix` recognised and refused.
const REFUSED_PROGRAMS: &[(&str, &str)] = &[
    ("nixos-rebuild", CHANGES_THE_MACHINE),
    ("nixos-install", CHANGES_THE_MACHINE),
    ("nixos-enter", RUNS_SOMETHING_ELSE),
    ("nix-env", CHANGES_THE_MACHINE),
    ("nix-collect-garbage", CHANGES_THE_MACHINE),
    ("nix-shell", RUNS_SOMETHING_ELSE),
    ("nix-daemon", OTHER_STORE),
    ("nix-copy-closure", OTHER_STORE),
    ("nix-channel", UNPINNED),
    ("nix-prefetch-url", UNPINNED),
];

/// `nix-store` operations that only query or realise the build's own store.
const STORE_OPERATIONS: &[&str] = &[
    "--realise",
    "-r",
    "--query",
    "-q",
    "--dump",
    "--export",
    "--verify-path",
    "--read-log",
    "-l",
    "--print-env",
];

/// Of [`STORE_OPERATIONS`], the ones that may substitute.
const STORE_FETCHING: &[&str] = &["--realise", "-r"];

/// Options that may come before a subcommand and take values, and how many.
///
/// Without these, `nix --store /build/nix build` would read `/build/nix` as the
/// subcommand. An option missing from here fails safe: its value is taken for
/// the subcommand, matches nothing, and the command gets no verdict.
const VALUED_OPTIONS: &[(&str, usize)] = &[
    ("--option", 2),
    ("--arg", 2),
    ("--argstr", 2),
    ("--override-input", 2),
    ("--store", 1),
    ("--eval-store", 1),
    ("--experimental-features", 1),
    ("--extra-experimental-features", 1),
    ("--log-format", 1),
    ("--max-jobs", 1),
    ("-j", 1),
    ("--cores", 1),
    ("--system", 1),
    ("-I", 1),
    ("--include", 1),
];

/// Paths NixOS fixes, for a build file that installs something referring to them.
///
/// Each is where every NixOS system puts it, not a choice this distribution made
/// -- the same test `losos-mkosi` applies. Only the store is visible inside pm's
/// build jail (pm mounts it read-only when it exists); the rest are run-time
/// paths, for a launcher or a unit a package installs.
const SYMBOLS: &[(&str, &str, &str)] = &[
    (
        "store",
        "/nix/store",
        "the Nix store; on this OS, the dm-verity /usr partition",
    ),
    (
        "current-system",
        "/run/current-system",
        "the running NixOS system's closure",
    ),
    (
        "booted-system",
        "/run/booted-system",
        "the NixOS system the machine booted, which an update does not move",
    ),
    (
        "system-bin",
        "/run/current-system/sw/bin",
        "where the running system's programs are on PATH",
    ),
];

/// The program name of a command, with any leading path removed.
///
/// pm allows an optional leading path in its own patterns (`/bin/sh`,
/// `./configure`), and a Nix-provisioned `nix` is usually spelled as a store
/// path, so a bare-name match would miss the commonest spelling.
fn program(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

/// The words of `args` that are not options or option values.
fn operands<'a>(args: &[&'a str]) -> Vec<&'a str> {
    let mut operands = Vec::new();
    let mut words = args.iter();
    while let Some(word) = words.next() {
        if !word.starts_with('-') {
            operands.push(*word);
            continue;
        }
        // `--store=/build/nix` carries its value in the same word.
        if word.contains('=') {
            continue;
        }
        let takes = VALUED_OPTIONS
            .iter()
            .find(|(name, _)| name == word)
            .map_or(0, |(_, n)| *n);
        for _ in 0..takes {
            words.next();
        }
    }
    operands
}

/// Classify the new `nix` command line.
fn decide_nix(args: &[&str]) -> Decision {
    let operands = operands(args);
    let Some(&subcommand) = operands.first() else {
        return Decision::Unknown;
    };
    let offline = args.contains(&"--offline");

    if let Some((_, reason)) = REFUSED.iter().find(|(name, _)| *name == subcommand) {
        return Decision::Refuse(reason);
    }

    let fingerprint = if LOCAL.contains(&subcommand) {
        return Decision::Classify {
            fingerprint: subcommand.into(),
            network: false,
        };
    } else if FETCHING.contains(&subcommand) {
        subcommand.to_string()
    } else if let Some((_, allowed)) = GROUPS.iter().find(|(name, _)| *name == subcommand) {
        let Some(&inner) = operands.get(1) else {
            return Decision::Unknown;
        };
        if subcommand == "flake" && REFUSED_FLAKE.contains(&inner) {
            return Decision::Refuse(UNPINNED);
        }
        if !allowed.contains(&inner) {
            return Decision::Unknown;
        }
        format!("{subcommand}-{inner}")
    } else {
        return Decision::Unknown;
    };

    if offline {
        Decision::Classify {
            fingerprint: format!("{fingerprint}-offline"),
            network: false,
        }
    } else {
        Decision::Classify {
            fingerprint,
            network: true,
        }
    }
}

/// Classify `nix-store`, whose operation is its first word after the program.
fn decide_nix_store(args: &[&str]) -> Decision {
    let Some(&operation) = args.first() else {
        return Decision::Unknown;
    };
    if !STORE_OPERATIONS.contains(&operation) {
        return if operation.starts_with('-') {
            // `--gc`, `--delete`, `--serve`, `--import`, `--optimise`, ...: every
            // other operation changes or serves the store.
            Decision::Refuse(OTHER_STORE)
        } else {
            Decision::Unknown
        };
    }
    Decision::Classify {
        fingerprint: "nix-store".into(),
        network: STORE_FETCHING.contains(&operation),
    }
}

/// What this plugin makes of `command`. Pure, so it can be tested off wasm.
fn decide(command: &str) -> Decision {
    let words: Vec<&str> = command.split_whitespace().collect();
    let Some((&first, args)) = words.split_first() else {
        return Decision::Unknown;
    };
    let program = program(first);

    if let Some((_, reason)) = REFUSED_PROGRAMS.iter().find(|(name, _)| *name == program) {
        return Decision::Refuse(reason);
    }

    match program {
        "nix" => decide_nix(args),
        "nix-store" => decide_nix_store(args),
        // Evaluation alone can fetch (`fetchTarball`, flake inputs), and neither
        // has an `--offline` that stops it.
        "nix-build" | "nix-instantiate" => Decision::Classify {
            fingerprint: program.into(),
            network: true,
        },
        "nix-hash" => Decision::Classify {
            fingerprint: program.into(),
            network: false,
        },
        _ => Decision::Unknown,
    }
}

impl Guest for LososNix {
    fn describe() -> Manifest {
        Manifest {
            name: "losos-nix".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            summary: "Classifies the Nix commands that read or build a flake".into(),
            // classify-command only. A .nix file does say what a package needs,
            // but only once evaluated, and a plugin has no evaluator; reading it
            // as text would be guessing, which is what losos-systemd exists to
            // improve on.
            hooks: vec![Hook::ClassifyCommand],
            // Network is in the ceiling because a flake build without offline
            // inputs needs it; Shell and VersionControl are not. Nix runs its
            // builders' shells inside its own sandbox, and reads a flake in a git
            // checkout through libgit2 rather than a git process.
            grants_at_most: vec![
                Capability::Toolchain,
                Capability::Coreutils,
                Capability::Archive,
                Capability::Network,
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
        match decide(&command) {
            Decision::Unknown => None,
            Decision::Refuse(reason) => {
                let program = command.split_whitespace().next().map(program)?;
                log(
                    Level::Info,
                    &format!("not classifying `{program}`: it {reason}"),
                );
                None
            }
            Decision::Classify {
                fingerprint,
                network,
            } => {
                // Archive because Nix unpacks what it fetches and packs NARs itself.
                let mut capabilities = vec![
                    Capability::Toolchain,
                    Capability::Coreutils,
                    Capability::Archive,
                ];
                if network {
                    capabilities.push(Capability::Network);
                }
                log(
                    Level::Debug,
                    &format!("classified `{command}` as {fingerprint}"),
                );
                Some(Verdict {
                    fingerprint,
                    capabilities,
                })
            }
        }
    }

    /// Not implemented, but the component model has no optional export, so it
    /// exists and returns the answer that changes nothing.
    fn scan_source(_file: SourceFile) -> Vec<Grant> {
        Vec::new()
    }
}

export!(LososNix);

#[allow(dead_code)]
fn _permission_is_used_by_the_world(_p: Permission) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn classified(fingerprint: &str, network: bool) -> Decision {
        Decision::Classify {
            fingerprint: fingerprint.into(),
            network,
        }
    }

    #[test]
    fn a_flake_build_gets_the_network_unless_it_says_offline() {
        assert_eq!(decide("nix build .#image"), classified("build", true));
        assert_eq!(
            decide("nix build --offline .#image"),
            classified("build-offline", false)
        );
    }

    #[test]
    fn a_store_path_program_and_leading_options_are_understood() {
        assert_eq!(
            decide(
                "/nix/store/abc-nix-2.31/bin/nix --store /build/nix --option sandbox false flake check"
            ),
            classified("flake-check", true)
        );
        assert_eq!(
            decide("nix --extra-experimental-features=flakes eval .#x"),
            classified("eval", true)
        );
    }

    #[test]
    fn an_unknown_option_value_fails_safe() {
        assert_eq!(decide("nix --builders ssh://x build"), Decision::Unknown);
    }

    #[test]
    fn reading_a_file_never_gets_the_network() {
        assert_eq!(decide("nix hash file x.tar"), classified("hash", false));
        assert_eq!(
            decide("nix-hash --type sha256 ."),
            classified("nix-hash", false)
        );
        assert_eq!(
            decide("nix-store --query --references /nix/store/a"),
            classified("nix-store", false)
        );
        assert_eq!(
            decide("nix-store -r /nix/store/a.drv"),
            classified("nix-store", true)
        );
    }

    #[test]
    fn machine_changes_runners_and_unpinned_fetches_are_refused() {
        assert_eq!(
            decide("nixos-rebuild switch --flake ."),
            Decision::Refuse(CHANGES_THE_MACHINE)
        );
        assert_eq!(
            decide("nix profile install .#pm"),
            Decision::Refuse(CHANGES_THE_MACHINE)
        );
        assert_eq!(
            decide("nix run .#pm"),
            Decision::Refuse(RUNS_SOMETHING_ELSE)
        );
        assert_eq!(
            decide("nix copy --to s3://x .#image"),
            Decision::Refuse(OTHER_STORE)
        );
        assert_eq!(decide("nix-store --gc"), Decision::Refuse(OTHER_STORE));
        assert_eq!(decide("nix flake update"), Decision::Refuse(UNPINNED));
        assert_eq!(
            decide("nix-prefetch-url https://x"),
            Decision::Refuse(UNPINNED)
        );
    }

    #[test]
    fn anything_else_is_left_to_pm() {
        assert_eq!(decide("nix"), Decision::Unknown);
        assert_eq!(decide("nix flake"), Decision::Unknown);
        assert_eq!(decide("nix store gc"), Decision::Unknown);
        assert_eq!(decide("nixfmt flake.nix"), Decision::Unknown);
    }
}
