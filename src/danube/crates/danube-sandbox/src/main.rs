//! danube-sandbox: what WebKit runs in bwrap's place to sandbox each web
//! process (lib.rs). It takes the bwrap options WebKit uses and builds the
//! same sandbox as a Hakoniwa container.
//!
//!     danube-sandbox [BWRAP OPTIONS] -- PROGRAM [ARGS...]

use std::io::IsTerminal;
use std::process::ExitCode;

use tracing_subscriber::EnvFilter;
use danube_sandbox::Plan;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--version") {
        // What a caller probing for bwrap expects to see.
        println!("bubblewrap 0.11.0 (danube-sandbox, Hakoniwa)");
        return ExitCode::SUCCESS;
    }
    let result = Plan::from_bwrap_args(args).and_then(danube_sandbox::run);
    match result {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{:?}", miette::Report::new(error));
            ExitCode::from(1)
        }
    }
}
