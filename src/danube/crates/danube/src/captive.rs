//! Captive portals: the sign-in page a hotel or train network shows before
//! it lets anything through.
//!
//! `danube captive-watch` runs in each session (a user service from
//! nixos/modules/danube.nix). Whenever systemd-networkd's state changes it
//! fetches the check URL (`losos.captivePortal.checkUrl`) over plain HTTP,
//! which a portal intercepts. An answer other than the expected one means a
//! portal is in the way, and it opens `danube --captive-portal`: one
//! window, no tabs, a throwaway profile and no content blockers, which
//! closes by itself once the check passes. The desktop entry's "Sign in to
//! network" action opens the same window by hand.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// What the check URL is expected to answer.
#[derive(Clone, Debug)]
pub struct Check {
    pub url: String,
    /// The body of a 200 answer when online; empty means a 204 is expected.
    pub expect: String,
}

impl Check {
    /// The check from `/etc/danube/captive-portal.conf` (`url = ...`,
    /// `expect = ...`), which the NixOS module writes.
    pub fn load() -> Option<Self> {
        let text = std::fs::read_to_string("/etc/danube/captive-portal.conf").ok()?;
        let mut check = Check {
            url: String::new(),
            expect: String::new(),
        };
        for (key, value) in text.lines().filter_map(|l| l.split_once('=')) {
            match key.trim() {
                "url" => check.url = value.trim().into(),
                "expect" => check.expect = value.trim().into(),
                _ => {}
            }
        }
        (!check.url.is_empty()).then_some(check)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Online,
    /// Something answered, but not what the check expects: a portal.
    Portal,
    /// No answer at all: offline, which no browser window can fix.
    Offline,
}

/// Splits `http://host[:port]/path` into its parts; only plain HTTP, the
/// only scheme a portal can intercept without a certificate error.
fn parse(url: &str) -> Option<(String, u16, String)> {
    let rest = url.strip_prefix("http://")?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse().ok()?),
        None => (authority, 80),
    };
    Some((host.to_owned(), port, path.to_owned()))
}

/// Reads an HTTP answer's status and body.
pub fn judge(response: &[u8], expect: &str) -> Verdict {
    let text = String::from_utf8_lossy(response);
    let Some((head, body)) = text.split_once("\r\n\r\n") else {
        return Verdict::Portal;
    };
    let status: u16 = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let online = if expect.is_empty() {
        status == 204
    } else {
        status == 200 && body.trim() == expect.trim()
    };
    if online {
        Verdict::Online
    } else {
        Verdict::Portal
    }
}

pub fn check(check: &Check) -> Verdict {
    let Some((host, port, path)) = parse(&check.url) else {
        return Verdict::Offline;
    };
    let Some(addr) = (host.as_str(), port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next())
    else {
        return Verdict::Offline;
    };
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_secs(5)) else {
        return Verdict::Offline;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let request = format!(
        "GET {path} HTTP/1.0\r\nHost: {host}\r\nUser-Agent: Danube\r\nConnection: close\r\n\r\n"
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return Verdict::Offline;
    }
    let mut response = Vec::new();
    // Enough for any check answer; a portal page past it changes nothing.
    let _ = stream.take(64 * 1024).read_to_end(&mut response);
    if response.is_empty() {
        return Verdict::Offline;
    }
    judge(&response, &check.expect)
}

/// The networkd state file, rewritten on every change of state.
const NETWORK_STATE: &str = "/run/systemd/netif/state";

/// Watches for network changes and opens the sign-in window when a portal
/// is in the way. Runs until killed.
pub fn watch(check_: Check, danube: &std::path::Path) -> ! {
    let mut last = None;
    loop {
        let state = std::fs::read_to_string(NETWORK_STATE).ok();
        if state != last {
            last = state.clone();
            let routable = state
                .as_deref()
                .is_some_and(|s| s.contains("OPER_STATE=routable"));
            // A just-joined network may take a moment to hand out DNS.
            std::thread::sleep(Duration::from_secs(2));
            if routable && check(&check_) == Verdict::Portal {
                tracing::info!("a captive portal is in the way; opening the sign-in window");
                match std::process::Command::new(danube)
                    .arg("--captive-portal")
                    .status()
                {
                    Ok(_) => {}
                    Err(error) => tracing::warn!(%error, "cannot open the sign-in window"),
                }
            }
        }
        std::thread::sleep(Duration::from_secs(3));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn judges_answers() {
        assert_eq!(
            judge(b"HTTP/1.1 204 No Content\r\n\r\n", ""),
            Verdict::Online
        );
        assert_eq!(
            judge(
                b"HTTP/1.1 302 Found\r\nLocation: http://portal/\r\n\r\n",
                ""
            ),
            Verdict::Portal
        );
        assert_eq!(
            judge(
                b"HTTP/1.1 200 OK\r\n\r\nNetworkManager is online\n",
                "NetworkManager is online"
            ),
            Verdict::Online
        );
        assert_eq!(
            judge(
                b"HTTP/1.1 200 OK\r\n\r\n<html>Log in</html>",
                "NetworkManager is online"
            ),
            Verdict::Portal
        );
    }

    #[test]
    fn parses_urls() {
        assert_eq!(
            parse("http://nmcheck.gnome.org/check_network_status.txt"),
            Some((
                "nmcheck.gnome.org".into(),
                80,
                "/check_network_status.txt".into()
            ))
        );
        assert_eq!(
            parse("http://a.example:8080"),
            Some(("a.example".into(), 8080, "/".into()))
        );
        assert_eq!(parse("https://a.example/"), None);
    }
}
