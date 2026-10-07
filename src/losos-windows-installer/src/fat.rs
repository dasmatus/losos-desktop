//! The XBOOTLDR partition's FAT filesystem, made without Windows.
//!
//! Windows would format it if asked, but only as a volume it has mounted,
//! and it mounts nothing from a partition of XBOOTLDR's type. So the
//! filesystem is written straight into the new partition, which Windows has
//! not mounted and never will, the same way the `/usr` halves are.
use std::fs;
use std::io::{self, Write};
use std::path::Path;

use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions};
use miette::{IntoDiagnostic, Result, WrapErr};

use crate::disk::{Device, Window};
use crate::install::EspFile;

/// Formats `len` bytes at `start` as FAT32 and copies `files` (path on the
/// new filesystem, local file) onto it.
pub fn format(
    device: &mut dyn Device,
    start: u64,
    len: u64,
    label: [u8; 11],
    files: &[(&str, &Path)],
) -> Result<()> {
    // The filesystem's sector is the disk's: Linux refuses to mount a FAT
    // whose sectors are smaller than the device's, which a 4Kn disk's are not.
    let sector = u16::try_from(device.sector_size()).into_diagnostic()?;
    {
        let window = Window::new(device, start, len);
        fatfs::format_volume(
            window,
            FormatVolumeOptions::new()
                .fat_type(FatType::Fat32)
                .bytes_per_sector(sector)
                .bytes_per_cluster(4096.max(u32::from(sector)))
                .volume_label(label),
        )
        .into_diagnostic()
        .wrap_err("formatting the boot partition")?;
    }
    let fs = FileSystem::new(Window::new(device, start, len), FsOptions::new())
        .into_diagnostic()
        .wrap_err("opening the boot partition")?;
    for (target, source) in files {
        let mut input = fs::File::open(source)
            .into_diagnostic()
            .wrap_err_with(|| source.display().to_string())?;
        let mut output = create(&fs, target)?;
        io::copy(&mut input, &mut output)
            .into_diagnostic()
            .wrap_err_with(|| format!("copying {} to the boot partition", source.display()))?;
        output.flush().into_diagnostic()?;
    }
    fs.unmount().into_diagnostic()?;
    Ok(())
}

/// Writes `files` onto an existing FAT filesystem at `start`, making the
/// directories they are in, and returns the paths that were not there
/// before. What `mountvol` and a copy do on Windows; this is a disk image's
/// way of doing the same.
pub fn put(
    device: &mut dyn Device,
    start: u64,
    len: u64,
    files: &[EspFile],
) -> Result<Vec<String>> {
    let fs = FileSystem::new(Window::new(device, start, len), FsOptions::new())
        .into_diagnostic()
        .wrap_err("opening the ESP")?;
    let mut created = Vec::new();
    for file in files {
        let existed = fs.root_dir().open_file(&file.path).is_ok();
        if existed && !file.replace {
            continue;
        }
        let mut output = create(&fs, &file.path)?;
        output.truncate().into_diagnostic()?;
        output.write_all(file.bytes).into_diagnostic()?;
        output.flush().into_diagnostic()?;
        if !existed {
            created.push(file.path.clone());
        }
    }
    fs.unmount().into_diagnostic()?;
    Ok(created)
}

/// Removes `paths` from the FAT filesystem at `start`, and the directories
/// that leaves empty. A path already gone is not an error.
pub fn remove(device: &mut dyn Device, start: u64, len: u64, paths: &[String]) -> Result<()> {
    let fs = FileSystem::new(Window::new(device, start, len), FsOptions::new())
        .into_diagnostic()
        .wrap_err("opening the ESP")?;
    for path in paths {
        let _ = fs.root_dir().remove(path);
        let mut parent = path.as_str();
        while let Some((dir, _)) = parent.rsplit_once('/') {
            // FAT refuses to remove a directory that still holds anything.
            if fs.root_dir().remove(dir).is_err() {
                break;
            }
            parent = dir;
        }
    }
    fs.unmount().into_diagnostic()?;
    Ok(())
}

/// Opens `path` for writing, creating it and every directory above it.
fn create<'a, T: fatfs::ReadWriteSeek>(
    fs: &'a FileSystem<T>,
    path: &str,
) -> Result<fatfs::File<'a, T>> {
    let mut dir = fs.root_dir();
    let mut parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let name = parts.pop().unwrap_or_default();
    for part in parts {
        dir = match dir.open_dir(part) {
            Ok(d) => d,
            Err(_) => dir
                .create_dir(part)
                .into_diagnostic()
                .wrap_err_with(|| format!("making {part}"))?,
        };
    }
    dir.create_file(name)
        .into_diagnostic()
        .wrap_err_with(|| format!("creating {path}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::RawDevice;
    use std::io::Read;

    #[test]
    fn formats_and_fills_a_partition() {
        let dir = std::env::temp_dir().join(format!("losos-fat-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let image = dir.join("disk.img");
        let uki = dir.join("uki.efi");
        let payload: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 256) as u8).collect();
        fs::write(&uki, &payload).unwrap();
        let file = fs::File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&image)
            .unwrap();
        file.set_len(600 << 20).unwrap();
        let mut device = RawDevice::new(file, 512);

        let start = 1 << 20;
        let len = 512 << 20;
        format(
            &mut device,
            start,
            len,
            *b"LOSOS BOOT ",
            &[("EFI/Linux/losos-desktop_1.efi", &uki)],
        )
        .unwrap();
        let esp = |path: &str, bytes: &'static [u8], replace| EspFile {
            path: path.into(),
            bytes,
            replace,
        };
        let created = put(
            &mut device,
            start,
            len,
            &[esp("loader/loader.conf", b"timeout 3\n", false)],
        )
        .unwrap();
        assert_eq!(created, vec!["loader/loader.conf".to_owned()]);
        // Not replaced, and not reported as created a second time.
        let created = put(
            &mut device,
            start,
            len,
            &[esp("loader/loader.conf", b"timeout 9\n", false)],
        )
        .unwrap();
        assert!(created.is_empty());

        let fs = FileSystem::new(Window::new(&mut device, start, len), FsOptions::new()).unwrap();
        assert_eq!(fs.fat_type(), FatType::Fat32);
        let mut back = Vec::new();
        fs.root_dir()
            .open_file("EFI/Linux/losos-desktop_1.efi")
            .unwrap()
            .read_to_end(&mut back)
            .unwrap();
        assert_eq!(back, payload);
        let mut conf = String::new();
        fs.root_dir()
            .open_file("loader/loader.conf")
            .unwrap()
            .read_to_string(&mut conf)
            .unwrap();
        assert_eq!(conf, "timeout 3\n");
        drop(fs);
        remove(&mut device, start, len, &["loader/loader.conf".into()]).unwrap();
        let fs = FileSystem::new(Window::new(&mut device, start, len), FsOptions::new()).unwrap();
        assert!(fs.root_dir().open_dir("loader").is_err());
        assert!(fs.root_dir().open_dir("EFI/Linux").is_ok());
        drop(fs);
        fs::remove_dir_all(&dir).unwrap();
    }
}
