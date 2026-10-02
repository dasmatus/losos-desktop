use std::fs;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::thread;

use losos_installer::disks::{self, Disk};
use losos_installer::install;
use losos_installer::network;
use losos_installer::wpa::{self, Control, Network, Security};

/// A scratch directory per test, removed when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("losos-installer-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn write(&self, path: &str, contents: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn scan_results_keep_the_strongest_of_each_ssid() {
    let reply = "bssid / frequency / signal level / flags / ssid\n\
        aa:aa:aa:aa:aa:01\t2412\t-70\t[WPA2-PSK-CCMP][ESS]\thome\n\
        aa:aa:aa:aa:aa:02\t5180\t-48\t[WPA2-PSK-CCMP][ESS]\thome\n\
        aa:aa:aa:aa:aa:03\t2437\t-60\t[ESS]\tcafe\n\
        aa:aa:aa:aa:aa:04\t2437\t-30\t[WPA2-PSK-CCMP][ESS]\t\n\
        aa:aa:aa:aa:aa:05\t2437\t-55\t[WPA2-PSK+SAE-CCMP][ESS]\tmy\\x20net\n\
        garbage\n";
    let networks = wpa::parse_scan_results(reply);
    let names: Vec<_> = networks.iter().map(|n| (n.name(), n.signal)).collect();
    assert_eq!(
        names,
        [
            ("home".to_string(), -48),
            ("my net".to_string(), -55),
            ("cafe".to_string(), -60)
        ]
    );
    assert_eq!(
        networks[1].security,
        Security::Passphrase {
            psk: true,
            sae: true
        }
    );
    assert_eq!(networks[2].security, Security::Open);
}

#[test]
fn ssids_decode_like_printf_encode() {
    assert_eq!(wpa::decode_ssid(r#"a\"b\\c\x41\xff"#), b"a\"b\\cA\xff");
    assert_eq!(wpa::decode_ssid(r"tab\there\n"), b"tab\there\n");
    // A truncated escape is kept as it was rather than dropped.
    assert_eq!(wpa::decode_ssid(r"end\x4"), b"end\\x4");
    assert_eq!(wpa::decode_ssid("trailing\\"), b"trailing\\");
}

#[test]
fn security_from_flags() {
    assert_eq!(
        Security::from_flags("[WPA2-EAP-CCMP][ESS]"),
        Security::Unsupported("802.1X")
    );
    assert_eq!(
        Security::from_flags("[WEP][ESS]"),
        Security::Unsupported("WEP")
    );
    assert_eq!(
        Security::from_flags("[RSN-SAE-CCMP][ESS]"),
        Security::Passphrase {
            psk: false,
            sae: true
        }
    );
    assert_eq!(Security::from_flags("[RSN-OWE-CCMP][ESS]"), Security::Owe);
    assert_eq!(Security::from_flags("[ESS]"), Security::Open);
}

#[test]
fn passphrase_settings() {
    let wpa2 = Security::Passphrase {
        psk: true,
        sae: false,
    };
    assert_eq!(
        wpa2.settings("correct horse").unwrap(),
        [
            ("key_mgmt", "WPA-PSK WPA-PSK-SHA256".to_string()),
            ("psk", "\"correct horse\"".to_string()),
            ("ieee80211w", "1".to_string()),
        ]
    );
    let raw = "0123456789abcdef".repeat(4);
    assert_eq!(wpa2.settings(&raw).unwrap()[1], ("psk", raw.clone()));
    assert!(wpa2.settings("short").is_err());
    assert!(wpa2.settings(&"x".repeat(64)).is_err());
    assert!(wpa2.settings("pässword").is_err());

    let wpa3 = Security::Passphrase {
        psk: false,
        sae: true,
    };
    let settings = wpa3.settings("pässwörd \"quoted\"").unwrap();
    assert_eq!(settings[0], ("key_mgmt", "SAE".to_string()));
    assert_eq!(
        settings[1],
        ("sae_password", "\"pässwörd \"quoted\"\"".to_string())
    );
    assert_eq!(settings[2], ("ieee80211w", "2".to_string()));
    assert!(wpa3.settings("").is_err());
    assert!(wpa3.settings("line\nbreak").is_err());

    let transition = Security::Passphrase {
        psk: true,
        sae: true,
    };
    let settings = transition.settings("12345678").unwrap();
    assert_eq!(
        settings[0],
        ("key_mgmt", "WPA-PSK WPA-PSK-SHA256 SAE".to_string())
    );
    assert_eq!(settings.last().unwrap(), &("ieee80211w", "1".to_string()));

    assert!(
        Security::Unsupported("802.1X")
            .settings("anything")
            .is_err()
    );
}

#[test]
fn status_and_online_state() {
    let status = wpa::parse_status("bssid=aa:aa:aa:aa:aa:01\nssid=home\nwpa_state=COMPLETED\n");
    assert!(status.contains(&("wpa_state".to_string(), "COMPLETED".to_string())));
    assert!(network::parse_online(
        "OPER_STATE=routable\nCARRIER_STATE=carrier\n"
    ));
    assert!(!network::parse_online("OPER_STATE=degraded\n"));
    assert!(!network::online(Path::new("/nonexistent/state")));
}

#[test]
fn wireless_interfaces_have_a_wireless_directory() {
    let sys = Scratch::new("net");
    fs::create_dir_all(sys.0.join("wlp2s0/wireless")).unwrap();
    fs::create_dir_all(sys.0.join("enp1s0")).unwrap();
    fs::create_dir_all(sys.0.join("lo")).unwrap();
    assert_eq!(wpa::interfaces(&sys.0), ["wlp2s0"]);
}

#[test]
fn control_socket_round_trip() {
    let dir = Scratch::new("ctrl");
    let server_path = dir.0.join("wlan0");
    let server = UnixDatagram::bind(&server_path).unwrap();
    let daemon = thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut seen = Vec::new();
        loop {
            let (n, from) = server.recv_from(&mut buf).unwrap();
            let command = String::from_utf8_lossy(&buf[..n]).into_owned();
            let from = from.as_pathname().unwrap().to_path_buf();
            let reply = match command.as_str() {
                "ADD_NETWORK" => "3\n",
                "SELECT_NETWORK 3" => {
                    // An unsolicited event first, which the client must skip.
                    server
                        .send_to(b"<3>CTRL-EVENT-SCAN-STARTED", &from)
                        .unwrap();
                    "OK\n"
                }
                c if c.starts_with("SET_NETWORK 3 ") => "OK\n",
                "QUIT" => break,
                _ => "FAIL\n",
            };
            seen.push(command);
            server.send_to(reply.as_bytes(), &from).unwrap();
        }
        seen
    });

    let client_dir = dir.0.join("client");
    fs::create_dir_all(&client_dir).unwrap();
    let control = Control::open_at(&server_path, &client_dir).unwrap();
    let network = Network {
        ssid: b"home".to_vec(),
        signal: -50,
        security: Security::Passphrase {
            psk: true,
            sae: false,
        },
    };
    assert_eq!(control.connect(&network, "correct horse").unwrap(), 3);
    assert!(control.forget(3).is_err());
    // The fake daemon stops answering at QUIT, so this one times out.
    control.request("QUIT").unwrap_err();
    let seen = daemon.join().unwrap();
    assert_eq!(
        seen,
        [
            "ADD_NETWORK",
            "SET_NETWORK 3 ssid 686f6d65",
            "SET_NETWORK 3 key_mgmt WPA-PSK WPA-PSK-SHA256",
            "SET_NETWORK 3 psk \"correct horse\"",
            "SET_NETWORK 3 ieee80211w 1",
            "SELECT_NETWORK 3",
            "REMOVE_NETWORK 3",
        ]
    );
    drop(control);
    assert_eq!(fs::read_dir(&client_dir).unwrap().count(), 0);
}

fn fake_disk(sys: &Scratch, name: &str, sectors: u64, model: &str) {
    sys.write(&format!("block/{name}/size"), &format!("{sectors}\n"));
    sys.write(
        &format!("block/{name}/device/model"),
        &format!("{model}   \n"),
    );
    sys.write(&format!("block/{name}/removable"), "0\n");
    sys.write(&format!("block/{name}/ro"), "0\n");
}

#[test]
fn disks_leave_out_what_cannot_be_installed_onto() {
    let sys = Scratch::new("disks");
    fake_disk(&sys, "sda", 1_000_215_216, "Samsung SSD 870");
    fake_disk(&sys, "sdb", 60_063_744, "Flash Disk");
    sys.write("block/sdb/removable", "1\n");
    fake_disk(&sys, "nvme0n1", 2_000_409_264, "WD_BLACK SN850X");
    fake_disk(&sys, "loop0", 2048, "");
    fake_disk(&sys, "sr0", 2048, "DVD");
    fake_disk(&sys, "mmcblk0boot0", 8192, "");
    fake_disk(&sys, "zram0", 8192, "");
    fake_disk(&sys, "sdc", 0, "Card reader");
    fake_disk(&sys, "sdd", 2048, "Write protected");
    sys.write("block/sdd/ro", "1\n");
    fake_disk(&sys, "nvme1c1n1", 2048, "multipath");
    sys.write("block/nvme1c1n1/hidden", "1\n");

    let found = disks::list(&sys.0, Some("sdb"));
    assert_eq!(
        found,
        [
            Disk {
                name: "nvme0n1".into(),
                path: "/dev/nvme0n1".into(),
                size: 2_000_409_264 * 512,
                model: "WD_BLACK SN850X".into(),
                removable: false,
            },
            Disk {
                name: "sda".into(),
                path: "/dev/sda".into(),
                size: 1_000_215_216 * 512,
                model: "Samsung SSD 870".into(),
                removable: false,
            },
        ]
    );
    assert!(
        disks::list(&sys.0, None)
            .iter()
            .any(|d| d.name == "sdb" && d.removable)
    );
    assert_eq!(found[1].size_text(), "512.1 GB");
    assert_eq!(disks::human_size(999), "999 B");
}

#[test]
fn the_medium_is_found_from_its_mount() {
    let sys = Scratch::new("medium");
    fs::create_dir_all(sys.0.join("block/sdb/sdb1")).unwrap();
    sys.write("block/sdb/sdb1/partition", "1\n");
    fs::create_dir_all(sys.0.join("block/sr0")).unwrap();
    fs::create_dir_all(sys.0.join("dev/block")).unwrap();
    std::os::unix::fs::symlink("../../block/sdb/sdb1", sys.0.join("dev/block/8:17")).unwrap();
    std::os::unix::fs::symlink("../../block/sr0", sys.0.join("dev/block/11:0")).unwrap();

    let stick = "22 1 0:21 / / rw - tmpfs tmpfs rw\n\
        35 22 8:17 / /iso ro,relatime - iso9660 /dev/sdb1 ro\n";
    assert_eq!(
        disks::backing_disk(stick, &sys.0, "/iso").as_deref(),
        Some("sdb")
    );
    let optical = "35 22 11:0 / /iso ro,relatime - iso9660 /dev/sr0 ro\n";
    assert_eq!(
        disks::backing_disk(optical, &sys.0, "/iso").as_deref(),
        Some("sr0")
    );
    assert_eq!(disks::backing_disk(stick, &sys.0, "/elsewhere"), None);
}

#[test]
fn esp_node_from_repart_json() {
    let json = r#"[
        {"type":"esp","label":"esp","uuid":"x","node":"/dev/sda1","activity":"create"},
        {"type":"usr-x86-64-verity","label":"_empty","node":"/dev/sda2"}
    ]"#;
    assert_eq!(install::esp_node(json).unwrap(), PathBuf::from("/dev/sda1"));
    assert!(install::esp_node(r#"[{"type":"usr-x86-64","node":"/dev/sda2"}]"#).is_err());
    assert!(install::esp_node("not json").is_err());
}

#[test]
fn transfers_name_the_chosen_disk() {
    let dir = Scratch::new("transfers");
    dir.write(
        "templates/20-usr.transfer",
        "[Target]\nType=partition\nPath=@TARGET@\n",
    );
    dir.write(
        "templates/30-uki.transfer",
        "[Target]\nType=regular-file\nPath=/run/esp/EFI/Linux\n",
    );
    dir.write("templates/README", "not a transfer @TARGET@\n");
    // A stale file from an earlier attempt must not survive into this one.
    dir.write("out/10-old.transfer", "stale\n");

    install::render_transfers(&dir.0.join("templates"), &dir.0.join("out"), "/dev/nvme0n1")
        .unwrap();
    let mut names: Vec<_> = fs::read_dir(dir.0.join("out"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["20-usr.transfer", "30-uki.transfer"]);
    assert_eq!(
        fs::read_to_string(dir.0.join("out/20-usr.transfer")).unwrap(),
        "[Target]\nType=partition\nPath=/dev/nvme0n1\n"
    );
    assert!(
        install::render_transfers(&dir.0.join("out/none"), &dir.0.join("o2"), "/dev/sda").is_err()
    );
}
