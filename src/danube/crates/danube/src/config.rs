//! Danube's settings: `key = value` lines, as derisk's own are, first
//! `/etc/danube/settings.conf`, which `losos.danube.*` writes, then the
//! person's `~/.config/danube/settings.conf`, whose lines win.
//!
//! There is no `search` key and no reading of what is typed: the window
//! has no address bar. Addresses are typed into derisk's command palette,
//! which opens an address as one and searches for anything else with the
//! desktop's engine, so Danube has nothing of its own to decide.

use std::fs;
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// What a new tab opens.
    pub home: String,
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
            jit: false,
        }
    }
}

impl Settings {
    /// The settings for the person whose home is `home`.
    pub fn load(home: &Path) -> Self {
        let mut settings = Self::default();
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

/// The host of `uri`, for permissions and the window's title; empty for a
/// URI without one.
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

    #[test]
    fn hosts() {
        assert_eq!(host("https://user@Meet.Example:443/room"), "meet.example");
        assert_eq!(host("http://[::1]:8080/"), "[::1]");
        assert_eq!(host("about:blank"), "");
    }

    #[test]
    fn settings_override_in_order() {
        let mut s = Settings::default();
        s.apply("javascript.jit = true\n# home = x\nhome = \n");
        assert!(s.jit);
        assert_eq!(s.home, "about:blank");
        s.apply("home = https://start.example/\nsearch = google\n");
        assert_eq!(s.home, "https://start.example/");
    }
}
