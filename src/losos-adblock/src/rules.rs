//! The DNS half of a filter list: which names to block, and which never to.
//!
//! A resolver sees a name and nothing else, so of an EasyList-style list it
//! can apply only the rules that mean "this whole domain": `||example.com^`,
//! which also covers every subdomain, and the same with `@@` in front as an
//! exception. A rule with a path, a wildcard or an option that narrows it to
//! some requests (`$third-party`, `$script`, `$domain=`) would block the whole
//! site at DNS level when the list's authors meant part of it, so it is left
//! to the web view's content blockers, which see URLs (webkit.rs). Hosts
//! files (`0.0.0.0 example.com`) and lists of bare domains, the two other
//! formats DNS blocklists come in, are read too; neither covers subdomains,
//! but a hosts list that wants them lists them.

use std::collections::HashSet;

/// What one line of a list says about DNS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rule {
    /// Block this name; `subdomains` when the rule covers them too.
    Block { name: String, subdomains: bool },
    /// Never block this name or anything under it.
    Allow(String),
}

/// Options that keep a network rule meaning the whole domain. `important`
/// only changes precedence inside a browser, and `all`, `document` and
/// `doc` widen a rule rather than narrow it.
const WHOLE_DOMAIN_OPTIONS: &[&str] = &["important", "all", "document", "doc", "popup"];

/// Addresses hosts files point blocked names at.
const SINKHOLES: &[&str] = &["0.0.0.0", "127.0.0.1", "::", "::1", "0", "fe80::1%lo0"];

/// Names a hosts file lists for the machine itself, which are never ads.
const LOCAL_NAMES: &[&str] = &[
    "localhost",
    "localhost.localdomain",
    "local",
    "broadcasthost",
    "ip6-localhost",
    "ip6-loopback",
    "ip6-localnet",
    "ip6-mcastprefix",
    "ip6-allnodes",
    "ip6-allrouters",
    "ip6-allhosts",
    "0.0.0.0",
];

/// The DNS meaning of one line, if it has one.
pub fn parse_line(line: &str) -> Option<Rule> {
    let line = line.trim();
    // `!` starts an adblock comment, `#` a hosts one; `[Adblock Plus 2.0]`
    // is a list header. `##` and `#@#` are element hiding, which only a
    // browser can do, and start with neither.
    if line.is_empty() || line.starts_with('!') || line.starts_with('[') {
        return None;
    }
    if line.contains("##") || line.contains("#@#") || line.contains("#?#") || line.contains("#$#") {
        return None;
    }
    if line.starts_with('#') {
        return None;
    }

    if let Some(rest) = line.strip_prefix("@@") {
        return network_rule(rest).map(|(name, _)| Rule::Allow(name));
    }
    if line.starts_with("||") || line.starts_with('|') {
        return network_rule(line).map(|(name, subdomains)| Rule::Block { name, subdomains });
    }

    // A hosts line: an address, then one or more names, then perhaps a
    // comment.
    let line = line.split('#').next().unwrap_or("").trim();
    let mut fields = line.split_whitespace();
    let first = fields.next()?;
    let rest: Vec<&str> = fields.collect();
    if rest.is_empty() {
        // A bare domain.
        return domain(first).map(|name| Rule::Block {
            name,
            subdomains: false,
        });
    }
    if !SINKHOLES.contains(&first) {
        // A hosts file that maps names to real addresses is not a blocklist.
        return None;
    }
    // Only the first name: hosts blocklists put one per line, and a line
    // with several is rare enough not to be worth a Vec per line.
    domain(rest[0]).map(|name| Rule::Block {
        name,
        subdomains: false,
    })
}

/// `||name^` with no options that narrow it: the domain, and whether it
/// covers subdomains (`||` does; `|http://name/` does not).
fn network_rule(rule: &str) -> Option<(String, bool)> {
    let (pattern, options) = match rule.split_once('$') {
        Some((pattern, options)) => (pattern, Some(options)),
        None => (rule, None),
    };
    if let Some(options) = options {
        let whole = options
            .split(',')
            .all(|option| WHOLE_DOMAIN_OPTIONS.contains(&option.trim()));
        if !whole {
            return None;
        }
    }
    let (rest, subdomains) = if let Some(rest) = pattern.strip_prefix("||") {
        (rest, true)
    } else if let Some(rest) = pattern.strip_prefix('|') {
        let rest = rest
            .strip_prefix("https://")
            .or_else(|| rest.strip_prefix("http://"))?;
        (rest, false)
    } else {
        return None;
    };
    // The domain ends at `^` (a separator), `/` with nothing after it, or
    // the end of the rule. Anything else after the name is a path.
    let end = rest.find(['^', '/']).unwrap_or(rest.len());
    let (name, tail) = rest.split_at(end);
    if !matches!(tail, "" | "^" | "/" | "^|" | "^/") {
        return None;
    }
    domain(name).map(|name| (name, subdomains))
}

/// A domain name as a resolver would compare it, or `None` for something
/// that is not one (a wildcard, an IP address, a name for this machine).
pub fn domain(name: &str) -> Option<String> {
    let name = name.trim().trim_end_matches('.').to_ascii_lowercase();
    if name.is_empty() || name.len() > 253 || LOCAL_NAMES.contains(&name.as_str()) {
        return None;
    }
    if !name.contains('.') {
        return None;
    }
    let valid = name.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    });
    if !valid || name.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    // Every label numeric is an IPv4 address written oddly.
    if name
        .split('.')
        .all(|label| label.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    Some(name)
}

/// Every name the lists block, and every name they, or the administrator,
/// exempt.
#[derive(Clone, Debug, Default)]
pub struct Blocklist {
    /// Blocked with their subdomains.
    trees: HashSet<String>,
    /// Blocked by exact name only (hosts files, bare domains).
    names: HashSet<String>,
    /// Never blocked, with their subdomains.
    allowed: HashSet<String>,
}

/// Why a name is blocked or not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Blocked by a rule for this name, or a parent.
    Blocked(String),
    /// A rule would block it, but this name, or a parent, is exempt.
    Allowed(String),
    /// No rule mentions it.
    Unlisted,
}

impl Blocklist {
    /// Adds one rule.
    pub fn add(&mut self, rule: Rule) {
        match rule {
            Rule::Block {
                name,
                subdomains: true,
            } => {
                self.trees.insert(name);
            }
            Rule::Block {
                name,
                subdomains: false,
            } => {
                self.names.insert(name);
            }
            Rule::Allow(name) => {
                self.allowed.insert(name);
            }
        }
    }

    /// Adds every rule in a list's text.
    pub fn add_list(&mut self, text: &str) {
        for rule in text.lines().filter_map(parse_line) {
            self.add(rule);
        }
    }

    /// How many names it blocks.
    pub fn len(&self) -> usize {
        self.trees.len() + self.names.len()
    }

    /// Whether it blocks nothing.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether `name` is blocked. An exemption for the name or any parent
    /// wins over a block: lists add `@@` rules for sites their own block
    /// rules break, and an administrator's allowed domain is meant to work.
    pub fn verdict(&self, name: &str) -> Verdict {
        let name = name.trim_end_matches('.').to_ascii_lowercase();
        let mut blocked = None;
        let mut suffix = name.as_str();
        loop {
            if self.allowed.contains(suffix) {
                return Verdict::Allowed(suffix.to_owned());
            }
            if blocked.is_none()
                && (self.trees.contains(suffix) || (suffix == name && self.names.contains(suffix)))
            {
                blocked = Some(suffix.to_owned());
            }
            match suffix.split_once('.') {
                Some((_, parent)) => suffix = parent,
                None => break,
            }
        }
        blocked.map_or(Verdict::Unlisted, Verdict::Blocked)
    }

    /// Writes the blocklist in the form [`Blocklist::read`] takes: one rule
    /// per line, `||name^` for a tree, the bare name for an exact one, and
    /// `@@||name^` for an exemption, so the compiled file is itself a valid
    /// filter list a person can read.
    pub fn write(&self) -> String {
        let mut lines: Vec<String> = Vec::with_capacity(self.len() + self.allowed.len());
        lines.extend(self.trees.iter().map(|n| format!("||{n}^")));
        lines.extend(self.names.iter().cloned());
        lines.extend(self.allowed.iter().map(|n| format!("@@||{n}^")));
        lines.sort_unstable();
        let mut text = lines.join("\n");
        text.push('\n');
        text
    }

    /// Reads what [`Blocklist::write`] wrote.
    pub fn read(text: &str) -> Self {
        let mut list = Self::default();
        list.add_list(text);
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(name: &str, subdomains: bool) -> Option<Rule> {
        Some(Rule::Block {
            name: name.into(),
            subdomains,
        })
    }

    #[test]
    fn reads_the_three_formats() {
        assert_eq!(
            parse_line("||ads.example.com^"),
            block("ads.example.com", true)
        );
        assert_eq!(
            parse_line("||Ads.Example.com^$important"),
            block("ads.example.com", true)
        );
        assert_eq!(
            parse_line("0.0.0.0 tracker.example.net"),
            block("tracker.example.net", false)
        );
        assert_eq!(
            parse_line("127.0.0.1\tt.example.org # why"),
            block("t.example.org", false)
        );
        assert_eq!(
            parse_line("metrics.example.io"),
            block("metrics.example.io", false)
        );
        assert_eq!(
            parse_line("@@||cdn.example.com^"),
            Some(Rule::Allow("cdn.example.com".into()))
        );
        assert_eq!(
            parse_line("|https://ads.example.com/"),
            block("ads.example.com", false)
        );
    }

    #[test]
    fn leaves_browser_rules_to_the_browser() {
        for line in [
            "||example.com/ads/*",
            "||example.com^$third-party",
            "||example.com^$script,domain=foo.com",
            "/banner/*/ad_",
            "example.com##.ad",
            "##.sponsored",
            "! comment",
            "# comment",
            "[Adblock Plus 2.0]",
            "192.168.1.10 nas.home.example",
            "0.0.0.0 localhost",
            "0.0.0.0 0.0.0.0",
            "||*.example.com^",
            "||1.2.3.4^",
        ] {
            assert_eq!(parse_line(line), None, "{line}");
        }
    }

    #[test]
    fn blocks_subdomains_only_for_tree_rules() {
        let list = Blocklist::read("||ads.example.com^\n0.0.0.0 t.example.net\n");
        assert_eq!(
            list.verdict("x.ads.example.com"),
            Verdict::Blocked("ads.example.com".into())
        );
        assert_eq!(
            list.verdict("ads.example.com."),
            Verdict::Blocked("ads.example.com".into())
        );
        assert_eq!(
            list.verdict("t.example.net"),
            Verdict::Blocked("t.example.net".into())
        );
        assert_eq!(list.verdict("x.t.example.net"), Verdict::Unlisted);
        assert_eq!(list.verdict("example.com"), Verdict::Unlisted);
    }

    #[test]
    fn exemptions_win() {
        let list = Blocklist::read("||example.com^\n@@||good.example.com^\n");
        assert_eq!(
            list.verdict("a.good.example.com"),
            Verdict::Allowed("good.example.com".into())
        );
        assert_eq!(
            list.verdict("bad.example.com"),
            Verdict::Blocked("example.com".into())
        );
    }

    #[test]
    fn round_trips() {
        let list = Blocklist::read("||a.example^\nb.example\n@@||c.example^\n");
        let again = Blocklist::read(&list.write());
        assert_eq!(again.len(), 2);
        assert_eq!(
            again.verdict("x.a.example"),
            Verdict::Blocked("a.example".into())
        );
        assert_eq!(again.verdict("x.b.example"), Verdict::Unlisted);
        assert_eq!(
            again.verdict("c.example"),
            Verdict::Allowed("c.example".into())
        );
    }
}
