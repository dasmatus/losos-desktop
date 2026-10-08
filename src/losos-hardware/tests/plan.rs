//! The plan for three machines: a laptop with an Intel CPU and iGPU, an
//! RTX 3050 and a Synaptics fingerprint reader; a desktop with an AMD CPU
//! and a GTX 1060 (Pascal, which NVIDIA's current driver no longer
//! supports); and a QEMU guest on an Intel host. The reports are in
//! nixos-facter's shape, cut down to the fields the rules read.

use std::fs;
use std::path::{Path, PathBuf};

use losos_hardware::{read_rules, Plan, Rule};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn report(name: &str) -> serde_json::Value {
    serde_json::from_slice(&fs::read(fixture(name)).unwrap()).unwrap()
}

/// The rule nixos/modules/nvidia.nix writes, with its device list in a file.
fn rules(dir: &Path) -> Vec<Rule> {
    let path = dir.join("rules.json");
    let rules = serde_json::json!({
        "rules": [{
            "name": "nvidia-open",
            "classes": ["graphics_card"],
            "vendor": 4318,
            "devices_file": fixture("nvidia-open-devices.json"),
            "load": ["nvidia", "nvidia_modeset", "nvidia-drm", "nvidia_uvm"],
            "blacklist": ["nouveau", "nova_core"],
            "fallback": ["nouveau"],
            "flags": ["nvidia"],
            "reserve": ["nvidia", "nvidia_modeset", "nvidia-drm", "nvidia_uvm"],
        }]
    });
    fs::write(&path, rules.to_string()).unwrap();
    read_rules(&path).unwrap()
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("losos-hardware-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_supported_nvidia_card_gets_the_open_driver() {
    let dir = tmp("ampere");
    let plan = Plan::new(&rules(&dir), &report("hybrid-ampere.json"));
    assert_eq!(plan.matched.len(), 1);
    // The NVIDIA card only; the Intel iGPU beside it keeps i915.
    assert_eq!(
        plan.matched[0].devices,
        ["0000:01:00.0 10de:25a2 nVidia GA107M [GeForce RTX 3050 Mobile]"]
    );

    // The rule matched, so nothing it reserves is kept off.
    assert!(plan.reserved.is_empty());

    plan.write(&dir).unwrap();
    let modprobe = fs::read_to_string(dir.join("modprobe.d/losos-hardware.conf")).unwrap();
    assert!(!modprobe.contains("blacklist nvidia"));
    assert!(modprobe.contains("blacklist nouveau\n"));
    assert!(modprobe.contains("blacklist nova_core\n"));
    let load = fs::read_to_string(dir.join("modules-load.d/losos-hardware.conf")).unwrap();
    assert!(load.ends_with("nvidia\nnvidia_modeset\nnvidia-drm\nnvidia_uvm\n"));
    assert!(dir.join("losos/hardware/flags/nvidia").exists());
    assert_eq!(Plan::read(&dir).unwrap(), plan);
}

#[test]
fn an_unsupported_nvidia_card_keeps_nouveau() {
    let dir = tmp("pascal");
    let plan = Plan::new(&rules(&dir), &report("pascal.json"));
    assert!(plan.matched.is_empty());

    plan.write(&dir).unwrap();
    let modprobe = fs::read_to_string(dir.join("modprobe.d/losos-hardware.conf")).unwrap();
    // nouveau is free to take it, and NVIDIA's modules, which also claim it
    // by alias, are kept off.
    assert!(!modprobe.contains("blacklist nouveau"));
    assert!(modprobe.contains("blacklist nvidia\n"));
    assert!(modprobe.contains("blacklist nvidia-drm\n"));
    let load = fs::read_to_string(dir.join("modules-load.d/losos-hardware.conf")).unwrap();
    assert!(load.lines().all(|l| l.starts_with('#')));
    assert!(!dir.join("losos/hardware/flags/nvidia").exists());
}

#[test]
fn devices_without_numeric_ids_are_skipped() {
    let report = report("qemu.json");
    // virtio's storage controller has names where hwinfo gives other devices
    // numbers; it is not a device any rule can match.
    assert_eq!(
        losos_hardware::devices(&report, "storage_controller").count(),
        0
    );
    assert_eq!(losos_hardware::devices(&report, "graphics_card").count(), 1);
    assert!(Plan::new(&rules(&tmp("qemu")), &report).matched.is_empty());
}

#[test]
fn a_second_run_replaces_the_first() {
    let dir = tmp("rerun");
    let rules = rules(&dir);
    Plan::new(&rules, &report("hybrid-ampere.json"))
        .write(&dir)
        .unwrap();
    Plan::new(&rules, &report("pascal.json"))
        .write(&dir)
        .unwrap();
    assert!(!dir.join("losos/hardware/flags/nvidia").exists());
    let modprobe = fs::read_to_string(dir.join("modprobe.d/losos-hardware.conf")).unwrap();
    assert!(!modprobe.contains("blacklist nouveau"));
}

#[test]
fn a_rule_whose_driver_did_not_load_has_failed() {
    let dir = tmp("failed");
    let plan = Plan::new(&rules(&dir), &report("hybrid-ampere.json"));
    let sys_module = dir.join("sys-module");
    for module in ["nvidia", "nvidia_modeset", "nvidia_uvm"] {
        fs::create_dir_all(sys_module.join(module)).unwrap();
    }
    // nvidia-drm is listed with a dash and shows up with an underscore.
    assert_eq!(plan.failed(&sys_module).count(), 1);
    fs::create_dir_all(sys_module.join("nvidia_drm")).unwrap();
    assert_eq!(plan.failed(&sys_module).count(), 0);
}

/// The service rules hardware.nix writes: thermald on Intel CPUs outside a
/// VM, fprintd with a reader libfprint drives, listed as vendor:device pairs
/// and found under whichever class hwinfo filed it.
fn service_rules(dir: &Path) -> Vec<Rule> {
    let path = dir.join("services.json");
    let readers = dir.join("readers.json");
    fs::write(&readers, r#"["06CB:00BD", "27C6:5110"]"#).unwrap();
    let rules = serde_json::json!({
        "rules": [
            {
                "name": "intel-thermald",
                "virtualisation": ["none"],
                "cpu_vendor": "GenuineIntel",
                "flags": ["intel-cpu"],
            },
            {
                "name": "fingerprint",
                "classes": ["fingerprint", "usb", "unknown"],
                "devices_file": readers,
                "flags": ["fingerprint"],
            },
            { "name": "matches-nothing" },
        ]
    });
    fs::write(&path, rules.to_string()).unwrap();
    read_rules(&path).unwrap()
}

fn rule_names(plan: &Plan) -> Vec<&str> {
    plan.matched.iter().map(|m| m.rule.as_str()).collect()
}

#[test]
fn services_follow_the_cpu_the_hypervisor_and_the_readers() {
    let dir = tmp("services");
    let rules = service_rules(&dir);

    let laptop = Plan::new(&rules, &report("hybrid-ampere.json"));
    assert_eq!(rule_names(&laptop), ["intel-thermald", "fingerprint"]);
    // Listed under both usb and fingerprint, reported once.
    assert_eq!(
        laptop.matched[1].devices,
        ["1-3 06cb:00bd Synaptics Prometheus MIS Touch Fingerprint Reader"]
    );
    assert_eq!(
        laptop.flags().into_iter().collect::<Vec<_>>(),
        ["fingerprint", "intel-cpu"]
    );

    // An AMD desktop without a reader: neither.
    assert!(Plan::new(&rules, &report("pascal.json")).matched.is_empty());
    // An Intel CPU, but inside a VM, where thermald has nothing to manage.
    assert!(Plan::new(&rules, &report("qemu.json")).matched.is_empty());
}

#[test]
fn without_a_report_the_reserved_modules_stay_off() {
    let dir = tmp("noreport");
    let plan = Plan::new(&rules(&dir), &serde_json::Value::Null);
    assert!(plan.matched.is_empty());
    assert_eq!(
        plan.reserved,
        ["nvidia", "nvidia_modeset", "nvidia-drm", "nvidia_uvm"]
    );
}
