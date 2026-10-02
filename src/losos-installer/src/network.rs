//! Whether the machine is online, as systemd-networkd sees it.
//!
//! networkd configures every link, wired and wireless alike (DHCP on both),
//! and writes its overall verdict to /run/systemd/netif/state. `routable`
//! there means some link has an address and a route beyond itself, which is
//! what downloading the image needs. A cable that was plugged in before boot
//! gets the machine here with nothing asked.
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
