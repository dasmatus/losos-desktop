//! Which disks the OS can be installed onto.
//!
//! Read from sysfs rather than lsblk, because every fact needed is a file
//! there and a fixture tree can stand in for it in the tests. The one disk
//! that is never offered is the one the installer itself is running from.
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Disk {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub model: String,
    pub removable: bool,
}

impl Disk {
    pub fn size_text(&self) -> String {
        human_size(self.size)
    }
}

/// Block devices that are not disks someone would install onto: loop and RAM
/// devices, optical drives, device-mapper and RAID assemblies (whose members
/// are offered instead), and an eMMC's boot and RPMB areas.
fn is_candidate(name: &str) -> bool {
    const SKIP: [&str; 9] = ["loop", "ram", "zram", "sr", "fd", "dm-", "md", "nbd", "zd"];
    if SKIP.iter().any(|prefix| name.starts_with(prefix)) {
        return false;
    }
    !(name.starts_with("mmcblk") && (name.contains("boot") || name.contains("rpmb")))
}

fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_owned())
}

/// Every disk under `sys/block`, except `exclude`.
pub fn list(sys: &Path, exclude: Option<&str>) -> Vec<Disk> {
    let mut disks: Vec<Disk> = fs::read_dir(sys.join("block"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| is_candidate(name) && Some(name.as_str()) != exclude)
        .filter_map(|name| {
            let dir = sys.join("block").join(&name);
            let flag = |file: &str| read_trimmed(&dir.join(file)).as_deref() == Some("1");
            // A read-only disk cannot be written, and a hidden one is an NVMe
            // multipath path whose namespace is listed under another name.
            if flag("ro") || flag("hidden") {
                return None;
            }
            // sysfs counts in 512-byte sectors whatever the disk's own sector
            // size is.
            let size = read_trimmed(&dir.join("size"))?.parse::<u64>().ok()? * 512;
            if size == 0 {
                return None;
            }
            let model = ["device/model", "device/name"]
                .iter()
                .find_map(|file| read_trimmed(&dir.join(file)).filter(|s| !s.is_empty()))
                .unwrap_or_default();
            Some(Disk {
                path: PathBuf::from("/dev").join(&name),
                removable: flag("removable"),
                name,
                size,
                model,
            })
        })
        .collect();
    disks.sort_by(|a, b| a.name.cmp(&b.name));
    disks
}

/// The disk holding the filesystem mounted at `mount_point`: the installer's
/// own medium, when given the ISO's mount point. mountinfo names the device by
/// number, and /sys/dev/block turns the number into a sysfs directory, which is
/// a partition's when the ISO was written to a stick and so has a parent.
pub fn backing_disk(mountinfo: &str, sys: &Path, mount_point: &str) -> Option<String> {
    let devno = mountinfo.lines().find_map(|line| {
        let fields: Vec<&str> = line.split(' ').collect();
        (fields.get(4) == Some(&mount_point)).then(|| fields[2].to_owned())
    })?;
    let dir = fs::canonicalize(sys.join("dev/block").join(devno)).ok()?;
    let disk = if dir.join("partition").exists() {
        dir.parent()?.to_path_buf()
    } else {
        dir
    };
    disk.file_name()?.to_str().map(str::to_owned)
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    // Decimal, as drives are sold.
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
