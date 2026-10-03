//! wpa_supplicant's control interface, spoken directly.
//!
//! The protocol is one datagram per command and one per reply over a Unix
//! socket, which is less code than parsing `wpa_cli`'s output and needs no
//! second process. Two details come from how NixOS runs the daemon: it is
//! unprivileged and sandboxed, with the control sockets under
//! /run/wpa_supplicant/control, and it can only answer a client socket that
//! sits under /run/wpa_supplicant/client and that its group may write. nixpkgs
//! patches `wpa_ctrl.c` to do exactly that for `wpa_cli`; [`Control::open_at`]
//! does the same here.
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

pub const CONTROL_DIR: &str = "/run/wpa_supplicant/control";
pub const CLIENT_DIR: &str = "/run/wpa_supplicant/client";

static CLIENTS: AtomicUsize = AtomicUsize::new(0);

pub struct Control {
    socket: UnixDatagram,
    local: PathBuf,
}

impl Control {
    pub fn open(interface: &str) -> io::Result<Self> {
        Self::open_at(
            &Path::new(CONTROL_DIR).join(interface),
            Path::new(CLIENT_DIR),
        )
    }

    pub fn open_at(remote: &Path, client_dir: &Path) -> io::Result<Self> {
        let local = client_dir.join(format!(
            "losos-installer-{}-{}",
            std::process::id(),
            CLIENTS.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_file(&local);
        let socket = UnixDatagram::bind(&local)?;
        let control = Control { socket, local };
        // A datagram reply needs write permission on the socket it is sent
        // to, and the daemon runs as its own user.
        if let Some(gid) = group_id(Path::new("/etc/group"), "wpa_supplicant") {
            let _ = std::os::unix::fs::chown(&control.local, None, Some(gid));
        }
        fs::set_permissions(&control.local, fs::Permissions::from_mode(0o660))?;
        control.socket.connect(remote)?;
        control
            .socket
            .set_read_timeout(Some(Duration::from_secs(5)))?;
        Ok(control)
    }

    pub fn request(&self, command: &str) -> io::Result<String> {
        self.socket.send(command.as_bytes())?;
        // SCAN_RESULTS is the longest reply, and wpa_supplicant caps every
        // reply at 4096 bytes itself.
        let mut buf = vec![0u8; 8192];
        loop {
            let n = self.socket.recv(&mut buf)?;
            let reply = String::from_utf8_lossy(&buf[..n]).into_owned();
            // An unsolicited event, which only an ATTACHed client receives.
            // This one never attaches; skipping them costs nothing if it did.
            if reply.starts_with('<') {
                continue;
            }
            return Ok(reply);
        }
    }

    /// A command whose only successful answer is `OK`.
    pub fn ok(&self, command: &str) -> io::Result<()> {
        let reply = self.request(command)?;
        if reply.trim_end() == "OK" {
            Ok(())
        } else {
            // The command line holds no secret worth hiding from the person who
            // typed it, but it is not needed to say what failed either.
            let verb = command
                .split_whitespace()
                .take(3)
                .collect::<Vec<_>>()
                .join(" ");
            Err(io::Error::other(format!(
                "wpa_supplicant refused {verb}: {}",
                reply.trim_end()
            )))
        }
    }

    pub fn status(&self) -> io::Result<Vec<(String, String)>> {
        Ok(parse_status(&self.request("STATUS")?))
    }

    /// Adds a network for `network` and selects it, which disables every
    /// other one. Returns the network id, for [`Control::forget`].
    pub fn connect(&self, network: &Network, passphrase: &str) -> io::Result<u32> {
        let reply = self.request("ADD_NETWORK")?;
        let id: u32 = reply.trim().parse().map_err(|_| {
            io::Error::other(format!("wpa_supplicant: ADD_NETWORK said {}", reply.trim()))
        })?;
        let set = |key: &str, value: &str| self.ok(&format!("SET_NETWORK {id} {key} {value}"));
        let result = (|| {
            // Hex, so an SSID may hold quotes, spaces or bytes that are not
            // UTF-8 without any quoting rule mattering.
            set("ssid", &hex(&network.ssid))?;
            for (key, value) in network.security.settings(passphrase)? {
                set(key, &value)?;
            }
            self.ok(&format!("SELECT_NETWORK {id}"))
        })();
        match result {
            Ok(()) => Ok(id),
            Err(e) => {
                let _ = self.forget(id);
                Err(e)
            }
        }
    }

    pub fn forget(&self, id: u32) -> io::Result<()> {
        self.ok(&format!("REMOVE_NETWORK {id}"))
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.local);
    }
}

fn group_id(group_file: &Path, name: &str) -> Option<u32> {
    let groups = fs::read_to_string(group_file).ok()?;
    groups.lines().find_map(|line| {
        let mut fields = line.split(':');
        (fields.next()? == name).then_some(())?;
        fields.nth(1)?.parse().ok()
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// How a network asks to be joined, read from the flags in a scan result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    Open,
    /// Opportunistic Wireless Encryption: encrypted, and no passphrase.
    Owe,
    /// WPA2-Personal, WPA3-Personal, or a transition network offering both.
    Passphrase {
        psk: bool,
        sae: bool,
    },
    /// 802.1X and WEP. The first needs more than a passphrase to describe,
    /// and the second is not worth describing; `wpa_cli` on another terminal
    /// can still join either.
    Unsupported(&'static str),
}

impl Security {
    pub fn from_flags(flags: &str) -> Self {
        if flags.contains("EAP") {
            Security::Unsupported("802.1X")
        } else if flags.contains("WEP") {
            Security::Unsupported("WEP")
        } else if flags.contains("PSK") || flags.contains("SAE") {
            Security::Passphrase {
                psk: flags.contains("PSK"),
                sae: flags.contains("SAE"),
            }
        } else if flags.contains("OWE") {
            Security::Owe
        } else {
            Security::Open
        }
    }

    pub fn needs_passphrase(self) -> bool {
        matches!(self, Security::Passphrase { .. })
    }

    pub fn label(self) -> &'static str {
        match self {
            Security::Open => "open",
            Security::Owe => "OWE",
            Security::Passphrase {
                psk: true,
                sae: true,
            } => "WPA2/WPA3",
            Security::Passphrase { sae: true, .. } => "WPA3",
            Security::Passphrase { .. } => "WPA2",
            Security::Unsupported(what) => what,
        }
    }

    /// The SET_NETWORK settings for this network beside its SSID.
    pub fn settings(self, passphrase: &str) -> io::Result<Vec<(&'static str, String)>> {
        match self {
            Security::Open => Ok(vec![("key_mgmt", "NONE".into())]),
            // OWE requires management frame protection by definition.
            Security::Owe => Ok(vec![("key_mgmt", "OWE".into()), ("ieee80211w", "2".into())]),
            Security::Passphrase { psk, sae } => {
                if passphrase.chars().any(char::is_control) {
                    return Err(io::Error::other(
                        "The passphrase cannot hold control characters.",
                    ));
                }
                let mut mgmt = Vec::new();
                let mut out = Vec::new();
                if psk {
                    // WPA2's passphrase is 8 to 63 printable ASCII characters;
                    // 64 hex digits are the raw key and go unquoted.
                    let raw =
                        passphrase.len() == 64 && passphrase.bytes().all(|b| b.is_ascii_hexdigit());
                    let ascii = passphrase.bytes().all(|b| (0x20..0x7f).contains(&b));
                    if !raw && !(ascii && (8..=63).contains(&passphrase.len())) {
                        return Err(io::Error::other(
                            "A WPA2 passphrase is 8 to 63 ASCII characters long.",
                        ));
                    }
                    mgmt.push("WPA-PSK WPA-PSK-SHA256");
                    out.push((
                        "psk",
                        if raw {
                            passphrase.to_owned()
                        } else {
                            format!("\"{passphrase}\"")
                        },
                    ));
                }
                if sae {
                    if passphrase.is_empty() {
                        return Err(io::Error::other("The passphrase is empty."));
                    }
                    mgmt.push("SAE");
                    // wpa_supplicant reads a quoted value up to its last quote,
                    // so a quote inside the passphrase survives.
                    out.push(("sae_password", format!("\"{passphrase}\"")));
                }
                // WPA3 alone requires protected management frames; a
                // transition network only offers them.
                let pmf = if sae && !psk { "2" } else { "1" };
                out.insert(0, ("key_mgmt", mgmt.join(" ")));
                out.push(("ieee80211w", pmf.into()));
                Ok(out)
            }
            Security::Unsupported(what) => Err(io::Error::other(format!(
                "{what} networks cannot be joined from here; use wpa_cli on another terminal."
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    pub ssid: Vec<u8>,
    /// dBm, the strongest of every access point broadcasting the SSID.
    pub signal: i32,
    pub security: Security,
}

impl Network {
    pub fn name(&self) -> String {
        String::from_utf8_lossy(&self.ssid).into_owned()
    }
}

/// SCAN_RESULTS: a header line, then one tab-separated line per access point
/// -- bssid, frequency, signal, flags, ssid. One network per SSID, strongest
/// first; hidden networks, which broadcast an empty SSID, are left out.
pub fn parse_scan_results(reply: &str) -> Vec<Network> {
    let mut networks: Vec<Network> = Vec::new();
    for line in reply.lines().skip(1) {
        let fields: Vec<&str> = line.splitn(5, '\t').collect();
        let [_, _, signal, flags, ssid] = fields[..] else {
            continue;
        };
        let Ok(signal) = signal.parse::<i32>() else {
            continue;
        };
        let ssid = decode_ssid(ssid);
        if ssid.iter().all(|&b| b == 0) {
            continue;
        }
        let network = Network {
            ssid,
            signal,
            security: Security::from_flags(flags),
        };
        match networks
            .iter_mut()
            .find(|n| n.ssid == network.ssid && n.security == network.security)
        {
            Some(known) if known.signal < network.signal => *known = network,
            Some(_) => {}
            None => networks.push(network),
        }
    }
    networks.sort_by(|a, b| b.signal.cmp(&a.signal).then_with(|| a.ssid.cmp(&b.ssid)));
    networks
}

/// Undoes wpa_supplicant's printf_encode(), which is how an SSID's raw bytes
/// reach a text protocol.
pub fn decode_ssid(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' || i + 1 == bytes.len() {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        i += 1;
        match bytes[i] {
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'e' => out.push(0x1b),
            b'x' => match bytes.get(i + 1..i + 3) {
                Some(&[hi, lo]) if hi.is_ascii_hexdigit() && lo.is_ascii_hexdigit() => {
                    out.push(hex_value(hi) << 4 | hex_value(lo));
                    i += 2;
                }
                _ => out.extend_from_slice(b"\\x"),
            },
            other => out.push(other),
        }
        i += 1;
    }
    out
}

fn hex_value(digit: u8) -> u8 {
    match digit {
        b'0'..=b'9' => digit - b'0',
        b'a'..=b'f' => digit - b'a' + 10,
        _ => digit - b'A' + 10,
    }
}

pub fn parse_status(reply: &str) -> Vec<(String, String)> {
    reply
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
}

/// Wireless interfaces are the network devices with a `wireless` directory.
pub fn interfaces(sys_class_net: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(sys_class_net)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().join("wireless").is_dir())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_lookup() {
        let dir =
            std::env::temp_dir().join(format!("losos-installer-group-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("group");
        fs::write(&file, "root:x:0:\nwpa_supplicant:x:991:\nwheel:x:1:a\n").unwrap();
        assert_eq!(group_id(&file, "wpa_supplicant"), Some(991));
        assert_eq!(group_id(&file, "nobody"), None);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn hex_ssid() {
        assert_eq!(hex(b"a \"b\""), "6120226222");
    }
}
