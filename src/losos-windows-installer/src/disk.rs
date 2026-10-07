//! Writing into partitions of a whole disk.
//!
//! Windows lets a program write a physical disk's sectors only in whole,
//! aligned sectors, and only outside any mounted filesystem. Every partition
//! this program writes is one it has just created, of a type Windows mounts
//! nothing from, so the second rule holds by construction; the first is
//! [`Device`]'s contract, and [`Window`] is the one place that turns
//! byte-sized reads and writes (a FAT filesystem's) into sector ones.
//!
//! The same code runs on Linux over a disk image file, which is how the
//! tests and the QEMU check in docs/windows-installer.md exercise it.
use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::ops::{Deref, DerefMut};
use std::path::Path;

use miette::{IntoDiagnostic, Result, WrapErr, bail};

/// A disk, read and written in whole sectors.
pub trait Device {
    fn sector_size(&self) -> u64;
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()>;
    fn write_at(&mut self, offset: u64, buf: &[u8]) -> io::Result<()>;
    fn flush(&mut self) -> io::Result<()>;
}

/// A disk opened as a file: `\\.\PhysicalDriveN` on Windows, an image on
/// Linux.
pub struct RawDevice {
    file: File,
    sector: u64,
}

impl RawDevice {
    pub fn new(file: File, sector: u64) -> RawDevice {
        RawDevice { file, sector }
    }

    fn check(&self, offset: u64, len: usize) -> io::Result<()> {
        if !offset.is_multiple_of(self.sector) || !(len as u64).is_multiple_of(self.sector) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "unaligned disk access: {len} bytes at {offset} on {}-byte sectors",
                    self.sector
                ),
            ));
        }
        Ok(())
    }
}

impl Device for RawDevice {
    fn sector_size(&self) -> u64 {
        self.sector
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.check(offset, buf.len())?;
        #[cfg(unix)]
        {
            std::os::unix::fs::FileExt::read_exact_at(&self.file, buf, offset)
        }
        #[cfg(windows)]
        {
            let mut done = 0;
            while done < buf.len() {
                let n = std::os::windows::fs::FileExt::seek_read(
                    &self.file,
                    &mut buf[done..],
                    offset + done as u64,
                )?;
                if n == 0 {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }
                done += n;
            }
            Ok(())
        }
    }

    fn write_at(&mut self, offset: u64, buf: &[u8]) -> io::Result<()> {
        self.check(offset, buf.len())?;
        #[cfg(unix)]
        {
            std::os::unix::fs::FileExt::write_all_at(&self.file, buf, offset)
        }
        #[cfg(windows)]
        {
            let mut done = 0;
            while done < buf.len() {
                let n = std::os::windows::fs::FileExt::seek_write(
                    &self.file,
                    &buf[done..],
                    offset + done as u64,
                )?;
                if n == 0 {
                    return Err(io::ErrorKind::WriteZero.into());
                }
                done += n;
            }
            Ok(())
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.sync_all()
    }
}

/// A zeroed buffer aligned for a disk opened without the cache: the storage
/// stack may refuse a buffer at an address the device cannot DMA from.
pub struct AlignedBuf {
    ptr: *mut u8,
    len: usize,
}

const ALIGN: usize = 4096;

impl AlignedBuf {
    pub fn new(len: usize) -> AlignedBuf {
        assert!(len > 0);
        let layout = Layout::from_size_align(len, ALIGN).expect("a valid layout");
        // SAFETY: the layout has a non-zero size.
        let ptr = unsafe { alloc_zeroed(layout) };
        if ptr.is_null() {
            std::alloc::handle_alloc_error(layout);
        }
        AlignedBuf { ptr, len }
    }
}

impl Deref for AlignedBuf {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        // SAFETY: ptr is a live allocation of len initialised bytes.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl DerefMut for AlignedBuf {
    fn deref_mut(&mut self) -> &mut [u8] {
        // SAFETY: as above, and &mut self makes the borrow unique.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.len) }
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        let layout = Layout::from_size_align(self.len, ALIGN).expect("a valid layout");
        // SAFETY: allocated in new() with this same layout.
        unsafe { dealloc(self.ptr, layout) }
    }
}

/// One partition of a [`Device`] as a byte-addressed file, for the FAT
/// library. A read or write that does not cover whole sectors reads the
/// sectors around it first.
pub struct Window<'a> {
    device: &'a mut dyn Device,
    start: u64,
    len: u64,
    pos: u64,
}

impl<'a> Window<'a> {
    pub fn new(device: &'a mut dyn Device, start: u64, len: u64) -> Window<'a> {
        Window {
            device,
            start,
            len,
            pos: 0,
        }
    }

    /// The sector-aligned span around `n` bytes at the current position.
    fn span(&self, n: usize) -> (u64, usize) {
        let sector = self.device.sector_size();
        let first = (self.start + self.pos) / sector * sector;
        let end = (self.start + self.pos + n as u64).div_ceil(sector) * sector;
        (first, (end - first) as usize)
    }

    fn clamp(&self, n: usize) -> usize {
        n.min((self.len.saturating_sub(self.pos)) as usize)
    }
}

impl Read for Window<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.clamp(buf.len().min(1 << 20));
        if n == 0 {
            return Ok(0);
        }
        let (first, span) = self.span(n);
        let mut sectors = AlignedBuf::new(span);
        self.device.read_at(first, &mut sectors)?;
        let skip = (self.start + self.pos - first) as usize;
        buf[..n].copy_from_slice(&sectors[skip..skip + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Write for Window<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.clamp(buf.len().min(1 << 20));
        if n == 0 {
            return if buf.is_empty() {
                Ok(0)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "write past the partition's end",
                ))
            };
        }
        let (first, span) = self.span(n);
        let mut sectors = AlignedBuf::new(span);
        let skip = (self.start + self.pos - first) as usize;
        let sector = self.device.sector_size() as usize;
        // Only the first and last sector can be partly covered.
        if skip != 0 || !n.is_multiple_of(sector) {
            self.device.read_at(first, &mut sectors)?;
        }
        sectors[skip..skip + n].copy_from_slice(&buf[..n]);
        self.device.write_at(first, &sectors)?;
        self.pos += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.device.flush()
    }
}

impl Seek for Window<'_> {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let pos = match to {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.len.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        match pos {
            Some(p) if p <= self.len => {
                self.pos = p;
                Ok(p)
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek outside the partition",
            )),
        }
    }
}

/// Counts what a reader has handed out, for progress.
struct Counted<R> {
    inner: R,
    count: u64,
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count += n as u64;
        Ok(n)
    }
}

/// Decompresses the `.raw.xz` at `path` into the partition at `start`, which
/// is `capacity` bytes long, and returns how many bytes it wrote. Progress is
/// reported as compressed bytes read out of the file's size.
pub fn write_xz(
    device: &mut dyn Device,
    start: u64,
    capacity: u64,
    path: &Path,
    mut progress: impl FnMut(u64, u64),
) -> Result<u64> {
    let file = File::open(path)
        .into_diagnostic()
        .wrap_err_with(|| path.display().to_string())?;
    let total = file.metadata().into_diagnostic()?.len();
    let mut input = Counted {
        inner: io::BufReader::with_capacity(1 << 20, file),
        count: 0,
    };
    // Every release file is xz's default multi-block form (image.nix runs
    // xz --threads), so liblzma can decode its blocks in parallel.
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get() as u32);
    let stream = liblzma::stream::MtStreamBuilder::new()
        .threads(threads)
        // Threads only while their buffers fit in 1 GiB; past that liblzma
        // decodes on one thread rather than stopping.
        .memlimit_threading(1 << 30)
        .memlimit_stop(u64::MAX)
        .decoder()
        .into_diagnostic()?;
    let mut decoder = liblzma::read::XzDecoder::new_stream(&mut input, stream);

    let sector = device.sector_size() as usize;
    let chunk = 4 << 20;
    let mut buffer = AlignedBuf::new(chunk);
    let mut written = 0u64;
    loop {
        // Fill the whole chunk, so every write but the last is a whole number
        // of sectors.
        let mut filled = 0;
        while filled < chunk {
            let n = decoder
                .read(&mut buffer[filled..])
                .into_diagnostic()
                .wrap_err_with(|| format!("decompressing {}", path.display()))?;
            if n == 0 {
                break;
            }
            filled += n;
        }
        if filled == 0 {
            break;
        }
        let padded = filled.div_ceil(sector) * sector;
        buffer[filled..padded].fill(0);
        if written + padded as u64 > capacity {
            bail!(
                "{} does not fit its {} byte partition",
                path.display(),
                capacity
            );
        }
        device
            .write_at(start + written, &buffer[..padded])
            .into_diagnostic()
            .wrap_err("writing to the disk")?;
        written += padded as u64;
        progress(decoder.get_ref().count, total);
        if filled < chunk {
            break;
        }
    }
    device.flush().into_diagnostic()?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(len: u64, sector: u64) -> (tempdir::Dir, RawDevice) {
        let dir = tempdir::Dir::new();
        let path = dir.0.join("disk.img");
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        file.set_len(len).unwrap();
        (dir, RawDevice::new(file, sector))
    }

    pub mod tempdir {
        use std::path::PathBuf;
        pub struct Dir(pub PathBuf);
        impl Dir {
            pub fn new() -> Dir {
                use std::sync::atomic::{AtomicU32, Ordering};
                static N: AtomicU32 = AtomicU32::new(0);
                let path = std::env::temp_dir().join(format!(
                    "losos-windows-installer-{}-{}",
                    std::process::id(),
                    N.fetch_add(1, Ordering::Relaxed)
                ));
                std::fs::create_dir_all(&path).unwrap();
                Dir(path)
            }
        }
        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn refuses_unaligned_access() {
        let (_dir, mut dev) = device(8192, 512);
        assert!(dev.write_at(1, &[0; 512]).is_err());
        assert!(dev.write_at(0, &[0; 100]).is_err());
        assert!(dev.write_at(512, &[0; 1024]).is_ok());
    }

    #[test]
    fn window_writes_bytes_through_whole_sectors() {
        let (_dir, mut dev) = device(16 * 4096, 4096);
        dev.write_at(4096, &[7u8; 4096]).unwrap();
        {
            let mut w = Window::new(&mut dev, 4096, 8192);
            w.seek(SeekFrom::Start(10)).unwrap();
            w.write_all(b"hello").unwrap();
            w.seek(SeekFrom::Start(4094)).unwrap();
            w.write_all(b"span").unwrap();
            assert!(w.seek(SeekFrom::Start(8193)).is_err());
            w.seek(SeekFrom::Start(8190)).unwrap();
            assert!(w.write_all(b"past the end").is_err());
        }
        let mut back = vec![0u8; 8192];
        dev.read_at(4096, &mut back).unwrap();
        assert_eq!(&back[..10], &[7u8; 10]);
        assert_eq!(&back[10..15], b"hello");
        assert_eq!(&back[15..4094], &[7u8; 4079][..]);
        assert_eq!(&back[4094..4098], b"span");
        let mut w = Window::new(&mut dev, 4096, 8192);
        w.seek(SeekFrom::Start(9)).unwrap();
        let mut got = [0u8; 7];
        w.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"\x07hello\x07");
    }

    #[test]
    fn writes_an_xz_image_into_its_partition() {
        let dir = tempdir::Dir::new();
        let raw: Vec<u8> = (0..3 * 1024 * 1024 + 100)
            .map(|i| (i % 251) as u8)
            .collect();
        let xz = dir.0.join("part.raw.xz");
        let mut encoder = liblzma::write::XzEncoder::new(File::create(&xz).unwrap(), 1);
        std::io::Write::write_all(&mut encoder, &raw).unwrap();
        encoder.finish().unwrap();

        let (_d, mut dev) = device(8 << 20, 512);
        let mut last = (0, 0);
        let written = write_xz(&mut dev, 1 << 20, 4 << 20, &xz, |a, b| last = (a, b)).unwrap();
        assert_eq!(written, (raw.len() as u64).div_ceil(512) * 512);
        assert_eq!(last.0, last.1);
        let mut full = vec![0u8; written as usize];
        dev.read_at(1 << 20, &mut full).unwrap();
        assert_eq!(&full[..raw.len()], &raw[..]);
        assert!(full[raw.len()..].iter().all(|&b| b == 0));

        // The same image into a partition one sector too small is refused.
        let small = (raw.len() as u64).div_ceil(512) * 512 - 512;
        assert!(write_xz(&mut dev, 1 << 20, small, &xz, |_, _| ()).is_err());
    }
}
