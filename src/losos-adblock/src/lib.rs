//! losos-adblock: ad and tracker blocking for every program on the OS, from
//! the same filter lists browsers use (docs/adblock.md).
//!
//! Two blockers share the lists. A DNS forwarder in front of resolved
//! refuses the domains they block, which covers every program, Flatpaks and
//! statically linked ones included, since they all resolve through
//! resolved's stub. A resolver only ever sees a name, though, and most of
//! EasyList is about URLs and page elements, so the same lists are also
//! compiled into WebKit content blockers that Danube loads.

pub mod dns;
pub mod lists;
pub mod rules;
pub mod serve;

use std::path::PathBuf;

/// Why a command failed.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum Error {
    #[error("{0} is neither an http(s) URL nor an absolute path")]
    #[diagnostic(
        code(losos_adblock::source),
        help("each line of /etc/losos-adblock/lists names one filter list")
    )]
    Source(String),
    #[error("cannot fetch {0}: {1}")]
    #[diagnostic(code(losos_adblock::fetch))]
    Fetch(String, String),
    #[error("cannot write {}", .0.display())]
    #[diagnostic(code(losos_adblock::write))]
    Write(PathBuf, #[source] std::io::Error),
    #[error("cannot listen for DNS queries")]
    #[diagnostic(
        code(losos_adblock::listen),
        help("run it from losos-adblock.socket, or pass --listen with a free address")
    )]
    Listen(#[source] std::io::Error),
}
