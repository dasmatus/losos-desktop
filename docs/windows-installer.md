# The Windows installer

`losos-desktop_<v>_x86_64-windows-installer.exe` installs LosOS beside
Windows, on the same disk, without a USB stick. It is a console program
(`src/losos-windows-installer`), built by `nixos/modules/windows-installer.nix`
from the OS's own configuration through nixpkgs' mingw-w64 cross toolchain,
and every nightly carries it beside the installer ISO
([Releases and GHCR](releases.md)). It is the
[ISO's installer](installer.md) for a machine that keeps Windows: it writes
what an update would, from the same channel, checked against the same key.

## What it does

Run as administrator (it asks for elevation itself if it was started
without), it:

1. Checks the machine. It refuses a Windows started in legacy BIOS mode, a
   machine of another architecture, Secure Boot turned on, and a drive that
   BitLocker is still encrypting or decrypting (below).
2. Finds the newest release on the channel compiled into it, the one
   `losos.update.baseUrl` names, and checks `SHA256SUMS.gpg` against the
   key in `nixos/keys/update-signing.asc`. `--release DIR` takes a release
   from a folder instead, such as a `LOSOS-RELEASE` disk.
3. Asks how much space LosOS gets, 64 GB unless the user types another
   number. The least is what first boot needs: the boot partition, two
   `/usr` slots, root's and `/home`'s minimums (`disk.nix`) and swap the
   size of RAM. The most leaves Windows its used space plus 10 GB, and never
   goes past what `Get-PartitionSupportedSize` says C: can shrink to.
4. Shows what it will do and waits for the word `install`. Nothing changes
   before that, and `--dry-run` stops there.
5. Downloads the UKI, `/usr` and its hash tree into
   `%ProgramData%\LosOS\Downloads` with Windows' own `curl.exe`, which
   resumes an interrupted download when the program is run again. Each file
   is checked against its line of `SHA256SUMS`.
6. Turns fast startup off, suspends BitLocker, and shrinks C: with
   `Resize-Partition`.
7. Adds three partitions in the space: a 512 MB FAT32 XBOOTLDR partition
   ("LosOS boot") holding the UKI, then slot A's verity and `/usr`
   partitions, written straight from the `.raw.xz` files. They are named,
   typed, flagged read-only and given the UUIDs the release's file names
   carry, exactly as sysupdate would write them, because that is how the
   initrd finds `/usr` from `usrhash=`.
8. Copies systemd-boot to `\EFI\systemd\` on Windows' EFI system partition,
   with `loader.conf` if there is none, and adds a firmware boot entry for
   it at the front of the boot order. The entry is Windows Boot Manager's
   own with the file swapped, so it points at the same disk the same way.
9. Offers to restart.

If any step after the shrink fails, it removes what it added and gives C:
its space back.

At the next start systemd-boot offers LosOS, from the XBOOTLDR partition,
and Windows Boot Manager, which it finds on the ESP by itself. LosOS's first
boot then does what it does after `dd`: `disk.nix` creates slot B, root,
`/home` and swap in the rest of the space, and first-boot setup asks for an
account. Windows' partitions are never touched again.

`--uninstall` takes it back: it deletes the partitions it finds behind C:
whose types are LosOS's, the files on the ESP it recorded in
`\loader\losos-windows-installer.txt`, and the boot entry, then grows C:
into the space.

## The layout

```
ESP                  Windows' own, ~100M: Windows Boot Manager, systemd-boot
MSR                  Windows' own
C:                   shrunk
LosOS boot           XBOOTLDR, 512M: the UKIs              installer
usr-verity A                                               installer
usr A                                                      installer
usr-verity B                                               first boot, _empty
usr B                                                      first boot, _empty
root, home, swap                                           first boot
Recovery             Windows' own, left where it was
```

Windows makes its ESP 100 MB, with its own partitions right behind it, so
it cannot grow and has no room for two UKIs of about 45 MB each. The UKIs
therefore go on an XBOOTLDR partition, which systemd-boot reads beside the
ESP, and `update.nix` writes them to `$BOOT`, which is that partition where
there is one and the ESP otherwise. `disk.nix` matches whatever ESP the disk
has and no longer asks for 512 MB of it: with that minimum, repart could not
grow Windows' ESP and refused the whole disk on first boot, so the system
never found its root. A release built before that change cannot be
installed beside Windows.

## Secure Boot, BitLocker, fast startup

- **Secure Boot** must be off. Neither systemd-boot nor the UKI is signed
  ([What is not done](not-done.md)), so firmware with Secure Boot on would
  refuse to start them, and a shim signed by Microsoft would need a key this
  project does not have. The installer says so and stops, before changing
  anything.
- **BitLocker** seals its key to the boot chain the TPM measured, and adding
  a boot manager in front of Windows' changes that chain. So the installer
  suspends protection on C: (`DisableKeyProtectors`) until Windows has
  started twice, which lets BitLocker reseal itself to the new chain the
  first time Windows comes up through systemd-boot. It still tells the user
  to have the recovery key at hand. A drive that is encrypting or
  decrypting is refused, because it cannot be shrunk safely.
- **Fast startup** hibernates Windows' kernel instead of shutting it down,
  with C: still mounted. LosOS's first boot changes the partition table
  around it, so the installer sets `HiberbootEnabled` to 0 before anything
  else. LosOS never mounts Windows' partitions, so this is about the
  partition table, not shared files.

## Testing it without Windows

The Linux build (`pkgs.losos-windows-installer`) is the same program but for
`windows.rs`, the part that talks to Windows. Its tests check the release's
real signature and manifest, the layout, the FAT and xz writers, and the
bytes of the firmware boot entry. With `--image DISK` it installs into a
disk image laid out like a Windows disk through `sfdisk` instead of Windows'
disk IOCTLs, leaving out the firmware entry. That is how the install was
checked: into a 64 GB image with a 100 MB ESP, an MSR, a C: and a recovery
partition, which then booted under QEMU into systemd-boot's menu with LosOS
and Windows Boot Manager, ran first boot, and left C: and recovery
byte-for-byte as they were.

What only a Windows machine can show is everything `windows.rs` does:
shrinking C:, writing the partition table through
`IOCTL_DISK_SET_DRIVE_LAYOUT_EX`, mounting the ESP, writing firmware
variables, BitLocker's suspension, and how a firmware treats the new boot
entry.
