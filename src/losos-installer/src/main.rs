//! `losos-installer serve`: the installer ISO's backend for `derisk
//! installer`, which runs it as a child and talks to it on stdin and stdout
//! (`serve.rs`).
//!
//! This used to be a terminal interface on tty1 that joined Wi-Fi, picked
//! the disk and showed the install. derisk now draws all of that as the
//! ISO's session, and joins networks itself the way an installed system
//! does, so only what is particular to LosOS stays here: which disks may be
//! installed onto, and repart and sysupdate run in order.
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use losos_installer::serve::{self, Config};
use miette::miette;
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage: losos-installer serve --repart-definitions DIR --sysupdate-definitions DIR \
[--esp DIR] [--medium DIR] [--work DIR] [--source URL] [--local-source DIR] [--name NAME]";

fn parse(mut args: impl Iterator<Item = String>) -> miette::Result<Config> {
    if args.next().as_deref() != Some("serve") {
        return Err(miette!(help = USAGE, "the only command is `serve`"));
    }
    let mut repart = None;
    let mut sysupdate = None;
    let mut config = Config {
        name: "LosOS Desktop".into(),
        repart_definitions: PathBuf::new(),
        sysupdate_templates: PathBuf::new(),
        esp: PathBuf::from("/run/losos-installer/esp"),
        medium: "/iso".into(),
        work: PathBuf::from("/run/losos-installer"),
        source: None,
        local_source: None,
    };
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| miette!(help = USAGE, "{flag} needs a value"))?;
        match flag.as_str() {
            "--repart-definitions" => repart = Some(PathBuf::from(value)),
            "--sysupdate-definitions" => sysupdate = Some(PathBuf::from(value)),
            "--esp" => config.esp = PathBuf::from(value),
            "--medium" => config.medium = value,
            "--work" => config.work = PathBuf::from(value),
            "--source" => config.source = Some(value),
            "--local-source" => config.local_source = Some(PathBuf::from(value)),
            "--name" => config.name = value,
            _ => return Err(miette!(help = USAGE, "unknown argument {flag}")),
        }
    }
    config.repart_definitions =
        repart.ok_or_else(|| miette!(help = USAGE, "--repart-definitions is required"))?;
    config.sysupdate_templates =
        sysupdate.ok_or_else(|| miette!(help = USAGE, "--sysupdate-definitions is required"))?;
    Ok(config)
}

fn main() -> ExitCode {
    // stderr, never stdout: stdout is the protocol `derisk installer` reads.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    match parse(std::env::args().skip(1)) {
        Ok(config) => {
            serve::serve_stdio(&config);
            ExitCode::SUCCESS
        }
        // Printed here rather than returned from `main`: a usage error exits
        // 2, as it always has, and `main() -> miette::Result` can only exit 1.
        Err(report) => {
            eprintln!("{report:?}");
            ExitCode::from(2)
        }
    }
}
