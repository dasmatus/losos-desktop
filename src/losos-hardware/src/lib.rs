//! Which drivers and services this machine gets, decided from
//! nixos-facter's report.
//!
//! The image is one build for every PC, so it carries drivers and services a
//! given machine has no use for. For most drivers that costs nothing: the
//! kernel names a module for each device it finds, and udev loads it. What
//! udev cannot do is choose. NVIDIA's open kernel module and nouveau both
//! claim every NVIDIA display controller, and which of them should drive a
//! card depends on which card it is; and a service such as fprintd or
//! thermald is only worth starting on a machine with a fingerprint reader or
//! an Intel CPU. NixOS's own answer, `hardware.facter`, reads the report
//! while the configuration is evaluated, which would mean a build per
//! machine; this reads the same report on the machine itself, at every boot,
//! and turns it into the files kmod and systemd already read from /run:
//!
//! - `modprobe.d/losos-hardware.conf` blacklists the drivers a matched rule
//!   displaces, so udev's coldplug does not load them;
//! - `modules-load.d/losos-hardware.conf` names the drivers it wants, which
//!   systemd-modules-load loads by name, past any blacklist;
//! - `losos/hardware/flags/<flag>` exists for each flag a rule sets, which
//!   units that only make sense on that hardware test with
//!   ConditionPathExists;
//! - `losos/hardware/plan.json` records what matched and why, which is also
//!   what `losos-hardware fallback` reads after the modules were loaded.
//!
//! The rules come from the NixOS configuration (nixos/modules/hardware.nix),
//! so the image's modules decide what hardware means, and this program only
//! matches.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// One choice the configuration makes about the machine.
///
/// A rule names what it matches by up to three selectors, all of which must
/// hold: a device (`classes`, with `vendor` and `devices`), the hypervisor
/// facter detected (`virtualisation`), and the CPU's vendor (`cpu_vendor`).
/// A rule with none of them never matches.
#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    /// Shown in the log and in plan.json.
    pub name: String,
    /// The facter hardware classes a matching device may be listed under,
    /// such as `graphics_card`, or `usb` and `fingerprint` for a reader that
    /// hwinfo may file under either.
    #[serde(default)]
    pub classes: Vec<String>,
    /// The PCI or USB vendor ID a matching device has.
    #[serde(default, deserialize_with = "vendor")]
    pub vendor: Option<u16>,
    /// IDs that match: a bare device ID, of `vendor`'s, or a `vvvv:dddd`
    /// pair. Every device in `classes` from `vendor` matches when this and
    /// `devices_file` are both absent.
    #[serde(default)]
    pub devices: Option<Ids>,
    /// The same as `devices`, read from a JSON array in a file, for lists
    /// that are build products: NVIDIA's table of the GPUs its driver
    /// supports, libfprint's of the readers it drives.
    #[serde(default)]
    pub devices_file: Option<PathBuf>,
    /// facter's `virtualisation` values that match, `none` for bare metal.
    #[serde(default)]
    pub virtualisation: Option<Vec<String>>,
    /// The CPU vendor string that matches, such as `GenuineIntel`.
    #[serde(default)]
    pub cpu_vendor: Option<String>,
    /// Modules to load when the rule matches.
    #[serde(default)]
    pub load: Vec<String>,
    /// Modules that must not bind the device when the rule matches.
    #[serde(default)]
    pub blacklist: Vec<String>,
    /// Modules to load instead when one of `load` did not come up, so a
    /// driver that refuses the card leaves the machine with the driver it
    /// would have had without the rule rather than with none.
    #[serde(default)]
    pub fallback: Vec<String>,
    /// Flags for units to test.
    #[serde(default)]
    pub flags: Vec<String>,
}

impl Rule {
    /// What in the report this rule matched, or None. A rule that matched
    /// without a device selector names the facts it matched on instead.
    fn matches(&self, report: &Value) -> Option<Vec<String>> {
        if self.classes.is_empty() && self.virtualisation.is_none() && self.cpu_vendor.is_none() {
            return None;
        }
        let mut found = Vec::new();
        if let Some(wanted) = &self.virtualisation {
            let virt = report["virtualisation"].as_str()?;
            wanted.iter().any(|w| w == virt).then_some(())?;
            found.push(format!("virtualisation {virt}"));
        }
        if let Some(wanted) = &self.cpu_vendor {
            report["hardware"]["cpu"]
                .as_array()?
                .iter()
                .any(|cpu| cpu["vendor_name"].as_str() == Some(wanted))
                .then_some(())?;
            found.push(format!("cpu {wanted}"));
        }
        if !self.classes.is_empty() {
            let before = found.len();
            // One device can be listed under two classes; say it once.
            let mut seen = BTreeSet::new();
            found.extend(
                self.classes
                    .iter()
                    .flat_map(|class| devices(report, class))
                    .filter(|d| self.vendor.is_none_or(|v| d.vendor == v))
                    .filter(|d| {
                        self.devices
                            .as_ref()
                            .is_none_or(|ids| ids.contains(self.vendor, d))
                    })
                    .filter(|d| seen.insert((d.bus_id.clone(), d.vendor, d.device)))
                    .map(|d| format!("{} {:04x}:{:04x} {}", d.bus_id, d.vendor, d.device, d.model)),
            );
            (found.len() > before).then_some(())?;
        }
        Some(found)
    }
}

/// Device IDs a rule lists: bare device IDs, which belong to the rule's
/// vendor, and vendor:device pairs.
#[derive(Debug, Clone, Default)]
pub struct Ids {
    devices: BTreeSet<u16>,
    pairs: BTreeSet<(u16, u16)>,
}

impl Ids {
    fn contains(&self, vendor: Option<u16>, d: &Device) -> bool {
        self.pairs.contains(&(d.vendor, d.device))
            || (vendor == Some(d.vendor) && self.devices.contains(&d.device))
    }

    fn extend(&mut self, other: Ids) {
        self.devices.extend(other.devices);
        self.pairs.extend(other.pairs);
    }
}

#[derive(Debug, Deserialize)]
struct RulesFile {
    rules: Vec<Rule>,
}

/// Reads the rules, and any device list each one names, so that matching
/// itself never touches the file system.
pub fn read_rules(path: &Path) -> io::Result<Vec<Rule>> {
    let RulesFile { rules } = serde_json::from_slice(&fs::read(path)?)?;
    rules
        .into_iter()
        .map(|mut rule| {
            if let Some(file) = rule.devices_file.take() {
                let listed: Ids = serde_json::from_slice(&fs::read(&file)?)?;
                rule.devices.get_or_insert_with(Ids::default).extend(listed);
            }
            Ok(rule)
        })
        .collect()
}

/// A device in facter's report, the fields the rules look at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Device {
    pub vendor: u16,
    pub device: u16,
    /// The PCI address or other bus ID, so the log says which device it was.
    pub bus_id: String,
    pub model: String,
}

/// The devices facter lists under `class`.
///
/// hwinfo gives a vendor or device as `{"hex": "10de", "value": 4318}` when
/// it read the number from the bus, and as a bare name for the devices it
/// made up itself, such as virtio's; those have no number to match, so they
/// are skipped rather than guessed at.
pub fn devices<'a>(report: &'a Value, class: &str) -> impl Iterator<Item = Device> + 'a {
    report["hardware"][class]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some(Device {
                vendor: number(&entry["vendor"])?,
                device: number(&entry["device"])?,
                bus_id: entry["sysfs_bus_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                model: entry["model"].as_str().unwrap_or_default().to_owned(),
            })
        })
}

fn number(field: &Value) -> Option<u16> {
    field["value"].as_u64().and_then(|n| u16::try_from(n).ok())
}

/// A rule that matched, with what it matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Matched {
    pub rule: String,
    pub devices: Vec<String>,
    pub load: Vec<String>,
    pub blacklist: Vec<String>,
    pub fallback: Vec<String>,
    pub flags: Vec<String>,
}

/// What this boot does about the machine's hardware.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub matched: Vec<Matched>,
}

impl Plan {
    /// Matches every rule against the report.
    pub fn new(rules: &[Rule], report: &Value) -> Self {
        let matched = rules
            .iter()
            .filter_map(|rule| {
                Some(Matched {
                    rule: rule.name.clone(),
                    devices: rule.matches(report)?,
                    load: rule.load.clone(),
                    blacklist: rule.blacklist.clone(),
                    fallback: rule.fallback.clone(),
                    flags: rule.flags.clone(),
                })
            })
            .collect();
        Self { matched }
    }

    /// The modprobe.d file: a blacklist line per displaced module.
    pub fn modprobe_conf(&self) -> String {
        let mut text = String::from(HEADER);
        for m in &self.matched {
            for module in &m.blacklist {
                text.push_str(&format!("blacklist {module}\n"));
            }
        }
        text
    }

    /// The modules-load.d file: a line per module to load.
    pub fn modules_load(&self) -> String {
        let mut text = String::from(HEADER);
        for m in &self.matched {
            for module in &m.load {
                text.push_str(&format!("{module}\n"));
            }
        }
        text
    }

    /// Every flag a matched rule sets, once each.
    pub fn flags(&self) -> BTreeSet<&str> {
        self.matched
            .iter()
            .flat_map(|m| m.flags.iter().map(String::as_str))
            .collect()
    }

    /// Writes the plan under `out`, which is /run on a running system.
    ///
    /// Every file is written even when nothing matched, and the flags are
    /// replaced rather than added to, so running it twice in one boot leaves
    /// what the second run decided.
    pub fn write(&self, out: &Path) -> io::Result<()> {
        let dir = out.join("losos/hardware");
        let flags = dir.join("flags");
        match fs::remove_dir_all(&flags) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        fs::create_dir_all(&flags)?;
        for flag in self.flags() {
            fs::write(flags.join(flag), "")?;
        }
        fs::write(dir.join("plan.json"), serde_json::to_vec_pretty(self)?)?;
        write(
            out.join("modprobe.d/losos-hardware.conf"),
            &self.modprobe_conf(),
        )?;
        write(
            out.join("modules-load.d/losos-hardware.conf"),
            &self.modules_load(),
        )
    }

    /// Reads back what `write` left under `out`.
    pub fn read(out: &Path) -> io::Result<Self> {
        Ok(serde_json::from_slice(&fs::read(
            out.join("losos/hardware/plan.json"),
        )?)?)
    }

    /// The matched rules one of whose modules is not loaded, by
    /// /sys/module (`sys_module`), where the kernel lists a module under its
    /// name with dashes made underscores.
    pub fn failed<'a>(&'a self, sys_module: &'a Path) -> impl Iterator<Item = &'a Matched> + 'a {
        self.matched.iter().filter(move |m| {
            m.load
                .iter()
                .any(|module| !sys_module.join(module.replace('-', "_")).exists())
        })
    }
}

const HEADER: &str =
    "# Written at boot by losos-hardware from this machine's nixos-facter report.\n";

fn write(path: PathBuf, text: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, text)
}

/// A vendor ID, as a number or as a hex string such as "0x10DE".
fn vendor<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u16>, D::Error> {
    Option::<Value>::deserialize(d)?
        .map(|v| parse_id(&v))
        .transpose()
        .map_err(serde::de::Error::custom)
}

impl<'de> Deserialize<'de> for Ids {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let mut ids = Ids::default();
        for value in Vec::<Value>::deserialize(d)? {
            match value.as_str().and_then(|s| s.split_once(':')) {
                Some((v, dev)) => {
                    let pair = (parse_id(&v.into()), parse_id(&dev.into()));
                    let (Ok(v), Ok(dev)) = pair else {
                        return Err(serde::de::Error::custom(format!(
                            "not a vendor:device pair: {value}"
                        )));
                    };
                    ids.pairs.insert((v, dev));
                }
                None => {
                    ids.devices
                        .insert(parse_id(&value).map_err(serde::de::Error::custom)?);
                }
            }
        }
        Ok(ids)
    }
}

/// An ID as a number, or as hex with or without "0x", which is how
/// NVIDIA's table and libfprint's list write them.
fn parse_id(value: &Value) -> Result<u16, String> {
    let parsed = match value {
        Value::Number(n) => n.as_u64().and_then(|n| u16::try_from(n).ok()),
        Value::String(s) => {
            let hex = s.trim_start_matches("0x").trim_start_matches("0X");
            u16::from_str_radix(hex, 16).ok()
        }
        _ => None,
    };
    parsed.ok_or_else(|| format!("not a 16-bit ID: {value}"))
}
