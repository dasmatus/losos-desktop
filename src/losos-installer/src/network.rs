//! Whether the machine is online, as systemd-networkd sees it, and what each
//! wired port is doing.
//!
//! networkd configures every link, wired and wireless alike (DHCP on both),
//! and writes its overall verdict to /run/systemd/netif/state. `routable`
//! there means some link has an address and a route beyond itself, which is
//! what downloading the image needs. A cable that was plugged in before boot
//! gets the machine here with nothing asked.
//!
//! The wired ports are listed only so the Network screen can say why it is
//! still offline: no cable, or a cable and no address yet.
use std::fs;
use std::path::Path;

pub fn online(state_file: &Path) -> bool {
    fs::read_to_string(state_file)
        .map(|state| parse_online(&state))
        .unwrap_or(false)
}

pub fn parse_online(state: &str) -> bool {
    state
        .lines()
        .filter_map(|line| line.split_once('='))
        .any(|(key, value)| key == "OPER_STATE" && value == "routable")
}

/// What a wired port is doing, in the order someone plugging in a cable
/// sees it happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WiredState {
    NoCable,
    Configuring,
    Online,
}

impl WiredState {
    pub fn label(self) -> &'static str {
        match self {
            WiredState::NoCable => "no cable",
            WiredState::Configuring => "cable in, getting an address",
            WiredState::Online => "online",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wired {
    pub name: String,
    pub state: WiredState,
}

/// The physical wired ports under `sys_class_net`, as the installer's
/// `20-wired` network matches them: Ethernet hardware (`type` 1, ARPHRD_ETHER)
/// with a device behind it, which leaves out loopback, bridges and veths,
/// and without a `wireless` directory, which leaves out Wi-Fi, whose type is
/// Ethernet too. Each one's state comes from its carrier and from networkd's
/// own file for it under `netif_links`, named by ifindex.
pub fn wired(sys_class_net: &Path, netif_links: &Path) -> Vec<Wired> {
    let mut ports: Vec<Wired> = fs::read_dir(sys_class_net)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let dir = entry.path();
            let read = |name: &str| {
                fs::read_to_string(dir.join(name))
                    .map(|s| s.trim().to_string())
                    .ok()
            };
            if read("type").as_deref() != Some("1")
                || !dir.join("device").exists()
                || dir.join("wireless").exists()
            {
                return None;
            }
            // Reading `carrier` fails while the link is down, which is the
            // same answer: nothing on the other end yet.
            let state = if read("carrier").as_deref() != Some("1") {
                WiredState::NoCable
            } else if read("ifindex")
                .and_then(|index| fs::read_to_string(netif_links.join(index)).ok())
                .is_some_and(|link| parse_online(&link))
            {
                WiredState::Online
            } else {
                WiredState::Configuring
            };
            Some(Wired {
                name: entry.file_name().into_string().ok()?,
                state,
            })
        })
        .collect();
    ports.sort_by(|a, b| a.name.cmp(&b.name));
    ports
}
