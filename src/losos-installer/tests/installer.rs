use std::fs;
use std::path::PathBuf;

use losos_installer::disks::{self, Disk};
use losos_installer::install;
use losos_installer::serve::{self, Request};

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

#[test]
fn download_progress_is_the_last_percentage() {
    assert_eq!(
        serve::percent("Got 45% of https://example/usr.raw"),
        Some(0.45)
    );
    assert_eq!(serve::percent("10% then 100%"), Some(1.0));
    assert_eq!(serve::percent("no number here"), None);
    assert_eq!(serve::percent("Got % of it"), None);
    assert_eq!(serve::percent("250%"), None);
}

#[test]
fn requests_parse_and_name_a_device() {
    assert_eq!(
        serve::parse_request(r#"{"method":"disks"}"#),
        Ok(Request::Disks)
    );
    assert_eq!(
        serve::parse_request(r#"{"method":"install","disk":"/dev/vda"}"#),
        Ok(Request::Install("/dev/vda".into()))
    );
    assert_eq!(
        serve::parse_request(r#"{"method":"reboot"}"#),
        Ok(Request::Reboot)
    );
    assert_eq!(
        serve::parse_request(r#"{"method":"power_off"}"#),
        Ok(Request::PowerOff)
    );
    assert!(serve::parse_request(r#"{"method":"install","disk":"vda"}"#).is_err());
    assert!(serve::parse_request(r#"{"method":"install"}"#).is_err());
    assert!(serve::parse_request(r#"{"method":"format"}"#).is_err());
    assert!(serve::parse_request("not json").is_err());
}

#[test]
fn steps_and_disks_as_derisk_reads_them() {
    let steps = serve::steps_event();
    assert_eq!(steps["event"], "steps");
    assert_eq!(
        steps["labels"].as_array().unwrap().len(),
        install::Step::ALL.len()
    );
    let step = serve::step_event(install::Step::Download, Some(0.5));
    assert_eq!(step["index"], 2);
    assert_eq!(step["count"], 4);
    assert_eq!(step["fraction"], 0.5);
    assert!(serve::step_event(install::Step::Partition, None)["fraction"].is_null());

    let disk = Disk {
        path: "/dev/vda".into(),
        name: "vda".into(),
        model: "QEMU".into(),
        size: 21_474_836_480,
        removable: false,
    };
    let event = serve::disks_event(&[disk]);
    assert_eq!(event["event"], "disks");
    assert_eq!(event["disks"][0]["path"], "/dev/vda");
    assert_eq!(event["disks"][0]["size"], 21_474_836_480u64);
}

#[test]
fn an_install_is_refused_for_a_disk_not_offered() {
    let config = serve::Config {
        name: "LosOS Desktop".into(),
        repart_definitions: PathBuf::from("/nonexistent"),
        sysupdate_templates: PathBuf::from("/nonexistent"),
        esp: PathBuf::from("/nonexistent"),
        medium: "/nonexistent".into(),
        work: PathBuf::from("/nonexistent"),
        source: None,
    };
    let input = "{\"method\":\"install\",\"disk\":\"/dev/not-a-disk\"}\nbogus\n";
    let (tx, rx) = std::sync::mpsc::channel();
    struct Sink(std::sync::mpsc::Sender<u8>);
    impl std::io::Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            buf.iter().for_each(|b| {
                let _ = self.0.send(*b);
            });
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serve::run(&config, input.as_bytes(), Sink(tx));
    let out = String::from_utf8(rx.iter().collect()).unwrap();
    let events: Vec<serde_json::Value> = out
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(events[0]["event"], "hello");
    assert_eq!(events[0]["name"], "LosOS Desktop");
    assert_eq!(events[1]["event"], "failed");
    assert!(
        events[1]["message"]
            .as_str()
            .unwrap()
            .contains("/dev/not-a-disk")
    );
    assert_eq!(events[2]["event"], "failed");
    assert_eq!(events.len(), 3);
}
