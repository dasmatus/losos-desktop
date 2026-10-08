//! Chooses this machine's drivers from its nixos-facter report.
//!
//!     losos-hardware plan --rules <file> --report <file> [--out /run]
//!     losos-hardware fallback [--out /run] [--sys-module /sys/module]
//!
//! `plan` runs at every boot before udev's coldplug and systemd-modules-load,
//! and writes what lib.rs describes. `fallback` runs after both: when a rule
//! wanted a module that is not loaded, it loads the rule's fallback, which is
//! the driver the machine would have had without the rule.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::Command;

use miette::{bail, IntoDiagnostic, WrapErr};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use losos_hardware::{read_rules, Plan};

fn main() -> miette::Result<()> {
    // stderr, which is the journal for a service.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .map(PathBuf::from)
    };
    let out = flag("--out").unwrap_or_else(|| "/run".into());

    match args.first().map(String::as_str) {
        Some("plan") => {
            let (Some(rules), Some(report)) = (flag("--rules"), flag("--report")) else {
                bail!("usage: losos-hardware plan --rules <file> --report <file> [--out <dir>]");
            };
            let rules = read_rules(&rules)
                .into_diagnostic()
                .wrap_err_with(|| format!("cannot read the rules in {}", rules.display()))?;
            let report: serde_json::Value = std::fs::read(&report)
                .into_diagnostic()
                .and_then(|bytes| serde_json::from_slice(&bytes).into_diagnostic())
                .wrap_err_with(|| format!("cannot read facter's report {}", report.display()))?;
            let plan = Plan::new(&rules, &report);
            for m in &plan.matched {
                info!(rule = %m.rule, devices = ?m.devices, load = ?m.load, blacklist = ?m.blacklist, "matched");
            }
            if plan.matched.is_empty() {
                info!("no rule matched; every device keeps the driver udev picks");
            }
            plan.write(&out)
                .into_diagnostic()
                .wrap_err_with(|| format!("cannot write the plan under {}", out.display()))
        }
        Some("fallback") => {
            let sys_module = flag("--sys-module").unwrap_or_else(|| "/sys/module".into());
            let plan = Plan::read(&out)
                .into_diagnostic()
                .wrap_err("cannot read this boot's plan")?;
            let mut ok = true;
            for m in plan.failed(&sys_module) {
                error!(rule = %m.rule, load = ?m.load, fallback = ?m.fallback, "a driver this rule wanted is not loaded; loading the fallback");
                if m.fallback.is_empty() {
                    continue;
                }
                // By name, so the blacklist the plan wrote does not stop it.
                let status = Command::new("modprobe")
                    .arg("-a")
                    .args(&m.fallback)
                    .status()
                    .into_diagnostic()
                    .wrap_err("cannot run modprobe")?;
                ok &= status.success();
            }
            if !ok {
                bail!("a fallback driver did not load either");
            }
            Ok(())
        }
        _ => bail!("usage: losos-hardware plan|fallback ..."),
    }
}
