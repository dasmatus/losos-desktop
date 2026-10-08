//! The plan for three machines: a laptop with an Intel iGPU and an RTX 3050,
//! a desktop with a GTX 1060 (Pascal, which NVIDIA's current driver no longer
//! supports), and a QEMU guest. The reports are in nixos-facter's shape, cut
//! down to the fields hwinfo fills for a PCI display controller.

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
            "class": "graphics_card",
            "vendor": 4318,
            "devices_file": fixture("nvidia-open-devices.json"),
            "load": ["nvidia", "nvidia_modeset", "nvidia-drm", "nvidia_uvm"],
            "blacklist": ["nouveau", "nova_core"],
            "fallback": ["nouveau"],
            "flags": ["nvidia"],
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

    plan.write(&dir).unwrap();
    let modprobe = fs::read_to_string(dir.join("modprobe.d/losos-hardware.conf")).unwrap();
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
    assert!(!modprobe.contains("blacklist"));
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
    assert!(!modprobe.contains("blacklist"));
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
