# Halium GSI

`nixos/halium/` is the phone and tablet target: one generic system image
(GSI) for every Android device that launched with Android 13 or later,
through [Halium](https://halium.org), the Android hardware layer that ports
such as Ubuntu Touch and Droidian build on. `nixosModules.gsi` imports
`nixos/modules/base.nix`, which holds everything a PC and a phone share (the
derisk desktop, homed accounts, networkd, pm, no Nix on the device, the `/etc`
overlay, sysusers), and adds its own boot (`boot.nix`) and Android layer
(`android.nix`) in place of the PC's UEFI, verity `/usr`, sysupdate and
installer. The [web flasher](web-flasher.md) installs it.

There are no device ports. The target used to be a configuration a port would
fill in with its kernel, device tree, `mkbootimg` offsets, partition labels and
udev rules; all of that is gone, because two things Android made generic cover
it:

- **Treble.** A device's vendor HALs run under any system image of a newer
  Android, so Halium's generic Android 14 system image starts them on every
  device. It is pinned in `nixos/pkgs/halium-gsi.nix` and lives in the store,
  so it updates with the OS.
- **GKI with `init_boot`.** A device that launched with Android 13 or later
  boots its maker's kernel from `boot`, its first drivers from `vendor_boot`,
  and the generic ramdisk from `init_boot`. The flasher replaces only that
  ramdisk, so the kernel, its modules and its device tree stay the device's
  own, and nothing here is built per device.

## How a device boots it

1. The bootloader loads the device's kernel, unpacks `vendor_boot`'s ramdisk
   and then `init_boot`'s, which is NixOS's systemd initrd, over it.
2. In the initrd, `losos-gsi-first-stage-modules` loads what `vendor_boot`
   lists in `/lib/modules/modules.load`: the storage controller and what it
   hangs off. Without them there is no `userdata`.
3. `userdata` is the root filesystem, whole, mounted by partition label and
   grown to the partition by `x-systemd.growfs` on the first boot. The command
   line comes from `boot` and `vendor_boot`, which this image does not write,
   so there is no `init=`; the initrd boots
   `/nix/var/nix/profiles/system`, which the flashed filesystem carries.
4. After switch-root, `losos-gsi-super` reads `super`'s logical partition
   metadata for the booted slot with `lpdump` and maps `vendor`, `odm`,
   `vendor_dlkm` and `system_dlkm` with device-mapper under their names
   without the slot suffix. They mount read-only at `/vendor`, `/odm`,
   `/vendor_dlkm` and `/system_dlkm`.
5. `losos-gsi-modules` points the kernel's firmware loader at
   `/vendor/firmware` and loads the generic kernel's modules from
   `system_dlkm`, then the vendor's. None of these partitions carries a
   `modules.dep.bin`, so the loader (`load-modules.sh`) runs `insmod` in
   passes until a pass loads nothing new, with `modules.options` and
   `modules.blocklist` applied; a module that never loads is logged and
   skipped.
6. Halium's system image is loop-mounted read-only from the store at
   `/android/system`, and `/android/system/system` at `/system`, where
   libhybris's linker looks.
7. `losos-android.service` starts Android's init with `lxc-start`. The
   container shares the host's `/dev` and network, gets the vendor partitions
   bind-mounted, and gets `/var/lib/android/data` as `/data`. It is LXC rather
   than `systemd-nspawn` because nspawn always gives a container a private
   `/dev`, and the HAL device nodes ueventd creates must be the ones the host
   opens. It starts before the display manager.
8. libhybris is a GLVND EGL vendor in `hardware.graphics`, beside Mesa.

The system carries no kernel (`boot.kernel.enable = false`): the one that
boots is the device's, so nixpkgs' would be dead weight in `rootfs`. Every
device in scope runs 5.10 or newer, which is systemd's minimum baseline.

Every mount on the Android side is `nofail`, so a device whose Android half is
missing or broken still reaches the login screen.

The container is not a security boundary. Android's init and the vendor HALs
run as host root with the host's whole `/dev`, every block device included,
so the vendor partition's closed blobs are trusted as much as the kernel, and
nothing here plays the part of the PC build's verity `/usr`. It drops
`sys_module`, `sys_rawio`, `sys_time` and the MAC capabilities; a `/dev`
holding only the HAL nodes would narrow it further, but which nodes those are
differs per device, so it waits for a device to find out.

## What it builds

`nix build .#packages.aarch64-linux.gsi` produces the three files the
flasher writes, with `SHA256SUMS`:

- `losos-desktop_<version>_gsi-init_boot.img`: the initrd as an Android boot
  image, header v4 with a ramdisk and nothing else, LZ4 in the legacy format
  the generic kernel decompresses.
- `losos-desktop_<version>_gsi-vbmeta.img`: an empty vbmeta with verification
  turned off, so a bootloader that checks `init_boot` against the stock
  vbmeta boots this one.
- `losos-desktop_<version>_gsi-userdata.simg.gz`: the root filesystem as an
  Android sparse image, gzipped because gzip is what a browser decompresses
  natively.

The aarch64 release carries the same three files under its `SHA256SUMS` and
signature, which is where the flasher downloads them from.

Without the flasher, the same three go on with fastboot:

```sh
fastboot flash vbmeta losos-desktop_<version>_gsi-vbmeta.img
fastboot flash init_boot losos-desktop_<version>_gsi-init_boot.img
gunzip losos-desktop_<version>_gsi-userdata.simg.gz
fastboot flash userdata losos-desktop_<version>_gsi-userdata.simg
fastboot reboot
```

## Which devices

A device is in scope when its bootloader can be unlocked and it has an
`init_boot` partition, which every device that launched with Android 13 or
later has. The flasher checks the second with `getvar
partition-size:init_boot_<slot>` and refuses a device without one.

Older devices, which launched before `init_boot` existed, boot a kernel and
ramdisk together from `boot`. A generic image cannot carry their kernel, so
each would need its own build, and they are not a target.

`nixos/pkgs/` builds `libhybris` from its upstream master and Halium's
`android-headers` (one tree serves Halium 11 to 16).
