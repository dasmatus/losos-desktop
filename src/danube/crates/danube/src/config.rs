//! Danube's settings and the address bar's reading of what is typed.
//!
//! Settings are `key = value` lines, as derisk's own are: first
//! `/etc/danube/settings.conf`, which `losos.danube.*` writes, then the
//! person's `~/.config/danube/settings.conf`, whose lines win. The search
//! engine is the desktop's: derisk's `defaults.search`, the one the choice
//! screen and Settings, Default apps set (`docs/choice-screens.md`), unless
//! Danube's own `search` names another.

use std::fs;
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// What a new tab opens.
    pub home: String,
    /// The search URL, the query appended to it.
    pub search: String,
    /// JavaScriptCore's JIT compilers. Off by default (docs/danube.md,
    /// "Sandbox and hardening"): the interpreter is slower, and a page
    /// can no longer have the browser write machine code for it, which is
    /// what most WebKit exploits rely on.
    pub jit: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            home: "about:blank".into(),
            search: search_url("duckduckgo").unwrap().into(),
            jit: false,
        }
    }
}

/// derisk's search engines (derisk-settings' `SearchEngine`), by the name
/// its settings file uses.
fn search_url(name: &str) -> Option<&'static str> {
    Some(match name {
        "duckduckgo" => "https://duckduckgo.com/?q=",
        "google" => "https://www.google.com/search?q=",
        "bing" => "https://www.bing.com/search?q=",
        "startpage" => "https://www.startpage.com/do/search?q=",
        "brave" => "https://search.brave.com/search?q=",
        "ecosia" => "https://www.ecosia.org/search?q=",
        "qwant" => "https://www.qwant.com/?q=",
        "mojeek" => "https://www.mojeek.com/search?q=",
        _ => return None,
    })
}

impl Settings {
    /// The settings for the person whose home is `home`.
    pub fn load(home: &Path) -> Self {
        let mut settings = Self::default();
        let derisk = fs::read_to_string(home.join(".config/derisk/settings.conf")).unwrap_or_default();
        for (key, value) in pairs(&derisk) {
            if key == "defaults.search" {
                if let Some(url) = search_url(value) {
                    settings.search = url.into();
                }
            }
        }
        for path in [
            Path::new("/etc/danube/settings.conf").to_path_buf(),
            home.join(".config/danube/settings.conf"),
        ] {
            let text = fs::read_to_string(path).unwrap_or_default();
            settings.apply(&text);
        }
        settings
    }

    fn apply(&mut self, text: &str) {
        for (key, value) in pairs(text) {
            match key {
                "home" if !value.is_empty() => self.home = value.into(),
                "search" => {
                    if let Some(url) = search_url(value) {
                        self.search = url.into();
                    } else if value.starts_with("https://") {
                        self.search = value.into();
                    }
                }
                "javascript.jit" => self.jit = value == "true",
                _ => {}
            }
        }
    }
}

fn pairs(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim(), v.trim()))
}

/// What to load for `typed` in the address bar: a URL as typed, a host
/// with https:// in front, or a search for anything else.
pub fn address(typed: &str, search: &str) -> Option<String> {
    let typed = typed.trim();
    if typed.is_empty() {
        return None;
    }
    let lower = typed.to_ascii_lowercase();
    if ["http://", "https://", "file://", "about:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
    {
        return Some(typed.into());
    }
    if !typed.contains(char::is_whitespace) {
        let host = typed.split(['/', '?', '#']).next().unwrap_or_default();
        let name = host.rsplit_once(':').map_or(host, |(name, port)| {
            if port.chars().all(|c| c.is_ascii_digit()) { name } else { host }
        });
        if name == "localhost" || name.parse::<std::net::Ipv4Addr>().is_ok() {
            return Some(format!("http://{typed}"));
        }
        let looks_like_host = name.contains('.')
            && !name.starts_with('.')
            && !name.ends_with('.')
            && name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '.');
        if looks_like_host {
            return Some(format!("https://{typed}"));
        }
    }
    Some(format!("{search}{}", encode(typed)))
}

/// A query string value: unreserved characters as they are, spaces as `+`,
/// everything else percent-encoded.
fn encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The host of `uri`, for permissions and the address bar; empty for a URI
/// without one.
pub fn host(uri: &str) -> String {
    let Some((_, rest)) = uri.split_once("://") else {
        return String::new();
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if authority.starts_with('[') {
        authority.split_inclusive(']').next().unwrap_or(authority)
    } else {
        authority.split(':').next().unwrap_or(authority)
    };
    host.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: &str = "https://duckduckgo.com/?q=";

    #[test]
    fn reads_what_is_typed() {
        assert_eq!(address("example.com", S).unwrap(), "https://example.com");
        assert_eq!(address("https://a.example/x", S).unwrap(), "https://a.example/x");
        assert_eq!(address("localhost:8080/x", S).unwrap(), "http://localhost:8080/x");
        assert_eq!(address("192.168.1.1", S).unwrap(), "http://192.168.1.1");
        assert_eq!(address("rust & wasm", S).unwrap(), "https://duckduckgo.com/?q=rust+%26+wasm");
        assert_eq!(address("hello", S).unwrap(), "https://duckduckgo.com/?q=hello");
        assert_eq!(address("  ", S), None);
    }

    #[test]
    fn hosts() {
        assert_eq!(host("https://user@Meet.Example:443/room"), "meet.example");
        assert_eq!(host("http://[::1]:8080/"), "[::1]");
        assert_eq!(host("about:blank"), "");
    }

    #[test]
    fn settings_override_in_order() {
        let mut s = Settings::default();
        s.apply("search = google\njavascript.jit = true\n# home = x\n");
        assert_eq!(s.search, "https://www.google.com/search?q=");
        assert!(s.jit);
        assert_eq!(s.home, "about:blank");
    }
}
