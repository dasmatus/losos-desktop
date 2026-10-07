//! losos-adblock's commands:
//!
//!     losos-adblock update           # fetch the lists and compile both blockers
//!     losos-adblock compile          # compile from the lists already fetched
//!     losos-adblock serve            # the DNS forwarder (socket-activated)
//!     losos-adblock check NAME...    # whether each name is blocked, and by what
//!
//! `--config DIR` and `--state DIR` replace /etc/losos-adblock and
//! /var/lib/losos-adblock, and `--listen ADDR` makes `serve` bind its own
//! sockets instead of taking systemd's.

use std::io::IsTerminal;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use losos_adblock::lists::{self, Paths};
use losos_adblock::rules::{Blocklist, Verdict};
use losos_adblock::serve::{self, Sockets};
use losos_adblock::Error;
use miette::IntoDiagnostic;
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage: losos-adblock [--config DIR] [--state DIR] update|compile|serve [--listen ADDR]|check NAME...";

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    match run() {
        Ok(code) => code,
        Err(report) => {
            eprintln!("{report:?}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> miette::Result<ExitCode> {
    let mut paths = Paths {
        config: PathBuf::from("/etc/losos-adblock"),
        state: PathBuf::from("/var/lib/losos-adblock"),
    };
    let mut listen: Option<SocketAddr> = None;
    let mut words = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => paths.config = args.next().ok_or_else(|| miette::miette!(USAGE))?.into(),
            "--state" => paths.state = args.next().ok_or_else(|| miette::miette!(USAGE))?.into(),
            "--listen" => {
                listen = Some(
                    args.next()
                        .ok_or_else(|| miette::miette!(USAGE))?
                        .parse()
                        .into_diagnostic()?,
                )
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(ExitCode::SUCCESS);
            }
            _ => words.push(arg),
        }
    }
    match words.first().map(String::as_str) {
        Some("update") => {
            let failed = lists::update(&paths)?;
            // Non-zero when a list could not be fetched, so the unit shows
            // it, though what could be compiled was.
            Ok(if failed == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            })
        }
        Some("compile") => {
            lists::compile(&paths)?;
            Ok(ExitCode::SUCCESS)
        }
        Some("serve") => {
            let sockets = match listen {
                Some(addr) => Sockets::bind(addr).map_err(Error::Listen)?,
                None => Sockets::from_systemd().ok_or_else(|| {
                    Error::Listen(std::io::Error::other("no sockets from systemd"))
                })?,
            };
            serve::run(sockets, paths.dns()).map_err(Error::Listen)?;
            Ok(ExitCode::SUCCESS)
        }
        Some("check") if words.len() > 1 => {
            let text = std::fs::read_to_string(paths.dns()).unwrap_or_default();
            let list = Blocklist::read(&text);
            let mut any = false;
            for name in &words[1..] {
                match list.verdict(name) {
                    Verdict::Blocked(rule) => {
                        any = true;
                        println!("{name}: blocked by ||{rule}^");
                    }
                    Verdict::Allowed(rule) => println!("{name}: allowed by @@||{rule}^"),
                    Verdict::Unlisted => println!("{name}: not blocked"),
                }
            }
            Ok(if any {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        _ => Err(miette::miette!(USAGE)),
    }
}
