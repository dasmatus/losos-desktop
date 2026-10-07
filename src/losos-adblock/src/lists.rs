//! Fetching the configured filter lists and compiling them for both
//! blockers: one domain list for the DNS forwarder, and WebKit content
//! blocker rule sets for Danube.
//!
//! The configuration is two files of one entry per line, `#` for comments:
//! `lists`, the URLs (or absolute paths) of the filter lists, and `allow`,
//! domains never to block. The OS writes them to /etc/losos-adblock from
//! `losos.adblock` (nixos/modules/adblock.nix); the same names in the state
//! directory add to them, for an administrator on a running system.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use tracing::{info, warn};

use crate::rules::{self, Blocklist, Rule};
use crate::Error;

/// WebKit refuses a rule set of more than 150,000 rules
/// (ContentExtensionParser.cpp), so a big list is split.
const RULES_PER_SET: usize = 100_000;

/// A list is never trusted to be larger than this; EasyList is about 2 MB.
const MAX_LIST_BYTES: u64 = 64 << 20;

/// Where the configuration and the compiled output live.
#[derive(Clone, Debug)]
pub struct Paths {
    /// /etc/losos-adblock: what the OS configured.
    pub config: PathBuf,
    /// /var/lib/losos-adblock: downloads, local additions, compiled output.
    pub state: PathBuf,
}

impl Paths {
    /// The compiled domain list the forwarder reads.
    pub fn dns(&self) -> PathBuf {
        self.state.join("dns.list")
    }

    /// The WebKit rule sets, one JSON file each, and `index` naming them.
    pub fn webkit(&self) -> PathBuf {
        self.state.join("webkit")
    }

    fn downloads(&self) -> PathBuf {
        self.state.join("lists")
    }
}

/// The entries of a config file in `config` and the same name in `state`.
fn entries(paths: &Paths, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    for dir in [&paths.config, &paths.state] {
        let Ok(text) = fs::read_to_string(dir.join(name)) else {
            continue;
        };
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if !line.is_empty() && !out.iter().any(|known| known == line) {
                out.push(line.to_owned());
            }
        }
    }
    out
}

/// A list's source, as the config names it.
pub fn sources(paths: &Paths) -> Vec<String> {
    entries(paths, "lists")
}

/// A stable file name for a source: its host and path made safe, and a hash
/// so two URLs differing only in characters dropped here stay apart.
pub fn slug(source: &str) -> String {
    let trimmed = source
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches('/');
    let mut name: String = trimmed
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    name.truncate(48);
    let name = name.trim_matches('-').to_owned();
    // FNV-1a: no dependency for a file name.
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in source.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{name}-{:08x}", hash as u32)
}

/// Writes `contents` to `path` so a reader never sees half of it.
pub fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("out")
    ));
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, path)
}

/// Fetches every list, keeping the last good copy of one that fails, then
/// compiles. Returns how many lists could not be fetched; compiling goes
/// ahead with what there is, so one list host being down blocks no less
/// than yesterday.
pub fn update(paths: &Paths) -> Result<usize, Error> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .user_agent("losos-adblock")
        .build()
        .into();
    let mut failed = 0;
    for source in sources(paths) {
        if let Err(error) = fetch(&agent, paths, &source) {
            warn!(%source, %error, "keeping the last copy of this list");
            failed += 1;
        }
    }
    compile(paths)?;
    Ok(failed)
}

/// Downloads one list, sending the ETag of the copy we have so an unchanged
/// list costs one round trip.
fn fetch(agent: &ureq::Agent, paths: &Paths, source: &str) -> Result<(), Error> {
    if source.starts_with('/') {
        // A local file is read where it is when compiling.
        return Ok(());
    }
    if !source.starts_with("https://") && !source.starts_with("http://") {
        return Err(Error::Source(source.to_owned()));
    }
    let base = paths.downloads().join(slug(source));
    let body_path = base.with_extension("txt");
    let etag_path = base.with_extension("etag");
    let mut request = agent.get(source);
    if body_path.exists() {
        if let Ok(etag) = fs::read_to_string(&etag_path) {
            request = request.header("If-None-Match", etag.trim());
        }
    }
    let response = request
        .call()
        .map_err(|e| Error::Fetch(source.to_owned(), e.to_string()))?;
    if response.status() == 304 {
        info!(%source, "unchanged");
        return Ok(());
    }
    let etag = response
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let mut body = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(MAX_LIST_BYTES)
        .read_to_end(&mut body)
        .map_err(|e| Error::Fetch(source.to_owned(), e.to_string()))?;
    let text = String::from_utf8_lossy(&body);
    // A captive portal or an error page answers 200 with HTML; that is not
    // a list, and replacing a good copy with it would unblock everything.
    if text.trim_start().starts_with('<') {
        return Err(Error::Fetch(
            source.to_owned(),
            "the response is HTML, not a filter list".into(),
        ));
    }
    write_atomic(&body_path, text.as_bytes()).map_err(|e| Error::Write(body_path.clone(), e))?;
    match etag {
        Some(etag) => write_atomic(&etag_path, etag.as_bytes()),
        None => fs::remove_file(&etag_path).or(Ok(())),
    }
    .map_err(|e| Error::Write(etag_path, e))?;
    info!(%source, bytes = body.len(), "fetched");
    Ok(())
}

/// The text of a source as last fetched, or of a local file.
fn read_source(paths: &Paths, source: &str) -> Option<String> {
    let path = if source.starts_with('/') {
        PathBuf::from(source)
    } else {
        paths.downloads().join(slug(source)).with_extension("txt")
    };
    fs::read_to_string(path).ok()
}

/// Builds the domain list and the WebKit rule sets from the lists on disk.
pub fn compile(paths: &Paths) -> Result<(), Error> {
    let mut dns = Blocklist::default();
    let mut webkit: BTreeMap<String, String> = BTreeMap::new();

    let allow: Vec<String> = entries(paths, "allow")
        .iter()
        .filter_map(|d| rules::domain(d))
        .collect();
    for name in &allow {
        dns.add(Rule::Allow(name.clone()));
    }
    // Exempt in the browser too. WebKit applies `ignore-previous-rules` only
    // within the rule set that carries it, so every set ends with it.
    let mut allow_rules = Vec::new();
    if !allow.is_empty() {
        let domains: Vec<String> = allow.iter().map(|d| format!("*{d}")).collect();
        allow_rules.push(serde_json::json!({
            "trigger": {"url-filter": ".*", "if-domain": domains},
            "action": {"type": "ignore-previous-rules"}
        }));
    }

    for source in sources(paths) {
        let Some(text) = read_source(paths, &source) else {
            warn!(%source, "not fetched yet; skipped");
            continue;
        };
        dns.add_list(&text);
        // The browser resolves through the forwarder too, so a rule the DNS
        // half already applies would only make WebKit compile it again; what
        // is left for WebKit is what needs a URL or a page.
        let for_webkit: String = text
            .lines()
            .filter(|line| !matches!(rules::parse_line(line), Some(Rule::Block { .. })))
            .flat_map(|line| [line, "\n"])
            .collect();
        for (n, set) in content_blockers(&for_webkit, &allow_rules)
            .into_iter()
            .enumerate()
        {
            webkit.insert(format!("{}-{n}", slug(&source)), set);
        }
    }

    let dns_path = paths.dns();
    write_atomic(&dns_path, dns.write().as_bytes()).map_err(|e| Error::Write(dns_path, e))?;
    info!(domains = dns.len(), "compiled the DNS blocklist");

    // Rule sets are written first and the index last, so a browser reading
    // the index finds every set it names.
    let dir = paths.webkit();
    fs::create_dir_all(&dir).map_err(|e| Error::Write(dir.clone(), e))?;
    for (id, set) in &webkit {
        let path = dir.join(format!("{id}.json"));
        write_atomic(&path, set.as_bytes()).map_err(|e| Error::Write(path, e))?;
    }
    let index: Vec<String> = webkit.keys().cloned().collect();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(id) = name.to_str().and_then(|n| n.strip_suffix(".json")) else {
                continue;
            };
            if !index.iter().any(|known| known == id) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    let index_path = dir.join("index");
    write_atomic(&index_path, index.join("\n").as_bytes())
        .map_err(|e| Error::Write(index_path, e))?;
    info!(sets = webkit.len(), "compiled the WebKit content blockers");
    Ok(())
}

/// One list as WebKit content blocker rule sets, each a JSON array, with
/// `trailing` appended to every set.
///
/// A list too big for one set is split, and WebKit applies a set's
/// `ignore-previous-rules` only to the rules before it in that same set, so
/// the list's exceptions (`@@` rules) are repeated at the end of every set
/// rather than landing in the last one alone.
pub fn content_blockers(text: &str, trailing: &[serde_json::Value]) -> Vec<String> {
    let mut set = adblock::lists::FilterSet::new(true);
    set.add_filter_list(text.to_owned(), adblock::lists::ParseOptions::default());
    let Ok((rules, _)) = set.into_content_blocking() else {
        return Vec::new();
    };
    let Ok(serde_json::Value::Array(rules)) = serde_json::to_value(&rules) else {
        return Vec::new();
    };
    let (exceptions, blocks): (Vec<_>, Vec<_>) = rules
        .into_iter()
        .partition(|rule| rule["action"]["type"] == "ignore-previous-rules");
    let mut tail = exceptions;
    tail.extend_from_slice(trailing);
    let per_set = RULES_PER_SET.saturating_sub(tail.len()).max(1);
    // A list of nothing but exceptions makes no set: there is nothing for
    // them to undo.
    blocks
        .chunks(per_set)
        .filter_map(|chunk| {
            let mut set = chunk.to_vec();
            set.extend_from_slice(&tail);
            serde_json::to_string(&set).ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_safe_and_distinct() {
        let a = slug("https://easylist.to/easylist/easylist.txt");
        assert!(a.starts_with("easylist-to-easylist-easylist-txt-"));
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        assert_ne!(a, slug("https://easylist.to/easylist/easylist.txt?x"));
    }

    #[test]
    fn converts_easylist_syntax() {
        let sets = content_blockers(
            "||ads.example.com^\n/banner/ad_\nexample.org##.sponsored\n",
            &[],
        );
        assert_eq!(sets.len(), 1);
        let rules: serde_json::Value = serde_json::from_str(&sets[0]).unwrap();
        let rules = rules.as_array().unwrap();
        assert!(rules.iter().any(|r| r["action"]["type"] == "block"));
        assert!(rules
            .iter()
            .any(|r| r["action"]["type"] == "css-display-none"));
    }

    #[test]
    fn exceptions_end_every_set() {
        let allow = serde_json::json!({"trigger": {"url-filter": ".*"}, "action": {"type": "ignore-previous-rules"}});
        let sets = content_blockers("/banner/ad_\n@@||good.example^\n", &[allow]);
        assert_eq!(sets.len(), 1);
        let rules: serde_json::Value = serde_json::from_str(&sets[0]).unwrap();
        let rules = rules.as_array().unwrap();
        assert_eq!(rules[0]["action"]["type"], "block");
        let last_two: Vec<_> = rules[rules.len() - 2..]
            .iter()
            .map(|r| &r["action"]["type"])
            .collect();
        assert!(last_two.iter().all(|t| *t == "ignore-previous-rules"));
    }

    #[test]
    fn compiles_from_disk() {
        let root = std::env::temp_dir().join(format!("losos-adblock-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let paths = Paths {
            config: root.join("etc"),
            state: root.join("state"),
        };
        fs::create_dir_all(&paths.config).unwrap();
        let list = root.join("list.txt");
        fs::write(
            &list,
            "||ads.example.com^\n0.0.0.0 t.example.net\n/banner/ad_\n",
        )
        .unwrap();
        fs::write(paths.config.join("lists"), format!("{}\n", list.display())).unwrap();
        fs::write(paths.config.join("allow"), "good.ads.example.com\n").unwrap();
        compile(&paths).unwrap();
        let dns = Blocklist::read(&fs::read_to_string(paths.dns()).unwrap());
        assert!(matches!(
            dns.verdict("x.ads.example.com"),
            rules::Verdict::Blocked(_)
        ));
        assert!(matches!(
            dns.verdict("good.ads.example.com"),
            rules::Verdict::Allowed(_)
        ));
        let index = fs::read_to_string(paths.webkit().join("index")).unwrap();
        // The DNS half covers the two domain rules; only the URL rule is
        // left for WebKit, and the allow list ends its set.
        assert_eq!(index.lines().count(), 1);
        let set = fs::read_to_string(paths.webkit().join(format!("{index}.json"))).unwrap();
        let rules: serde_json::Value = serde_json::from_str(&set).unwrap();
        let rules = rules.as_array().unwrap();
        assert_eq!(rules[0]["action"]["type"], "block");
        assert_eq!(
            rules[rules.len() - 1]["action"]["type"],
            "ignore-previous-rules"
        );
        let _ = fs::remove_dir_all(&root);
    }
}
