//! A security report for a Linux desktop, in the shape GNOME already renders.
//!
//! fwupd's Host Security ID answers whether the *firmware* is trustworthy.
//! That is a real question and not the one a desktop user is asking, which is
//! nearer "is this machine set up the way a careful person would set it up" --
//! TPM, disk encryption, kernel hardening, boot integrity. lynis answers that
//! for servers, as a shell script producing a report nobody reads twice.
//!
//! This is the OS-level half, emitted in fwupd's own attribute shape so
//! GNOME's Privacy & Security panel lists it alongside the firmware checks
//! rather than in a second application. A security report split across two
//! applications is two reports.
//!
//! Three ways to run it:
//!
//!     losos-security                 # human-readable, exit 0/1 on pass/fail
//!     losos-security --json          # the same, machine-readable
//!     losos-security --serve         # D-Bus, for the Settings panel
//!
//! `--root <dir>` points every check at a fixture tree instead of the running
//! system, which is how the suite is tested without a TPM.

use std::collections::HashMap;
use std::io::IsTerminal;
use std::process::ExitCode;

use miette::{IntoDiagnostic, WrapErr};
use tracing::info;
use tracing_subscriber::EnvFilter;
use zbus::{connection, interface, zvariant::OwnedValue};

use losos_security::attr::Attr;
use losos_security::checks::{self, Context};

struct Security {
    root: String,
}

#[interface(name = "io.losos.Security1")]
impl Security {
    /// Deliberately the same method name and signature fwupd uses.
    ///
    /// The panel change that consumes this is "ask a second bus name and
    /// concatenate": no new parsing, no new widget, and if this service is
    /// absent the panel shows exactly what it showed before.
    async fn get_host_security_attrs(&self) -> Vec<HashMap<String, OwnedValue>> {
        let ctx = Context::new(&self.root);
        checks::run(&ctx).iter().map(Attr::to_dbus).collect()
    }

    /// The weakest-link level across every check.
    #[zbus(property)]
    async fn host_security_id(&self) -> String {
        let ctx = Context::new(&self.root);
        let attrs = checks::run(&ctx);
        format!("LOSOS:{}", checks::overall_level(&attrs))
    }
}

fn report(attrs: &[Attr]) -> bool {
    let mut ok = true;
    for a in attrs {
        let mark = if a.success { "pass" } else { "FAIL" };
        if !a.success {
            ok = false;
        }
        println!("  {mark}  [HSI:{}] {}", a.hsi_level, a.summary);
        println!("        {}", a.evidence);
    }
    println!("\n  overall: LOSOS:{}", checks::overall_level(attrs));
    if !ok {
        // The level is the weakest link, so say which link. A report that
        // prints a number and leaves the reader to find the failure is a
        // report that gets glanced at once.
        println!("  failing:");
        for a in attrs.iter().filter(|a| !a.success) {
            println!("    {} -- {}", a.summary, a.evidence);
        }
    }
    ok
}

#[tokio::main]
async fn main() -> miette::Result<ExitCode> {
    // stderr, so `--json` output on stdout stays parseable; a service's
    // stderr is the journal.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();

    let args: Vec<String> = std::env::args().collect();
    let root = args
        .iter()
        .position(|a| a == "--root")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "/".to_string());

    if args.iter().any(|a| a == "--serve") {
        let security = Security { root };
        let _conn = connection::Builder::system()
            .and_then(|b| b.name("io.losos.Security1"))
            .and_then(|b| b.serve_at("/io/losos/Security1", security))
            .into_diagnostic()
            .wrap_err("cannot reach the system bus")?
            .build()
            .await
            .into_diagnostic()
            .wrap_err("cannot take io.losos.Security1")?;
        info!("serving io.losos.Security1");

        // Socket-activated and idle-exiting would be better, but a report that
        // is cheap to produce and rarely asked for is not worth the machinery.
        //
        // Both signals, not just SIGINT: systemd stops a service with SIGTERM,
        // and a process that waits only for ctrl_c relies on the default
        // disposition killing it -- which works, and leaves the connection to
        // be torn down by the kernel rather than by this process. Waiting for
        // either means `systemctl stop` is an ordinary exit.
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .into_diagnostic()
            .wrap_err("cannot listen for SIGTERM")?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
        return Ok(ExitCode::SUCCESS);
    }

    let ctx = Context::new(&root);
    let attrs = checks::run(&ctx);

    if args.iter().any(|a| a == "--json") {
        let text = serde_json::to_string_pretty(&attrs).into_diagnostic()?;
        println!("{text}");
        return Ok(ExitCode::SUCCESS);
    }

    // A failing check is a report, not an error: it exits 1 with no
    // diagnostic, the report on stdout saying which check failed.
    Ok(if report(&attrs) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
