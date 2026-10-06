# Halium

`nixos/halium/` is a second target: the same OS on a phone or tablet through
[Halium](https://halium.org), the Android hardware layer that ports such as
Ubuntu Touch and Droidian build on. `nixosModules.halium` imports
`nixos/modules/base.nix`, which holds everything a PC and a phone share (the
derisk desktop, homed accounts, networkd, pm, no Nix on the device, the `/etc`
overlay, sysusers), and adds its own boot and Android layer in place of the
PC's UEFI, verity `/usr`, sysupdate and installer.

How a device boots it:

1. The Android bootloader loads `boot.img` from the boot partition. It holds
   the device's kernel and NixOS's systemd initrd, with `init=` naming the
   system as the UKI does on a PC. Halium's own rootfs boots through
   halium-boot, a busybox ramdisk; this keeps systemd in the initrd instead.
2. The initrd mounts `userdata` at `/run/halium/userdata`, grows
   `losos/rootfs.img` on it to `losos.halium.rootfsSize`, and loop-mounts it
   as `/` with `x-systemd.growfs`.
3. After switch-root, the Halium system image (system-as-root, `/init` at its
   top) is mounted read-only at `/android/system`, `vendor` at `/vendor`, and
   `/android/system/system` at `/system`, where libhybris's linker looks.
4. `losos-android.service` starts Android's init with `lxc-start`. The
   container shares the host's `/dev` and network and gets userdata as
   `/data`, as in Halium's lxc-android. It is LXC rather than
   `systemd-nspawn` because nspawn always gives a container a private `/dev`,
   and the HAL device nodes ueventd creates must be the ones the host opens.
   It starts before the display manager.
5. libhybris is a GLVND EGL vendor in `hardware.graphics`, beside Mesa.

Every mount on the Android side is `nofail`, so a device whose Android half is
missing or broken still reaches the login screen.

The container is not a security boundary. Android's init and the vendor HALs
run as host root with the host's whole `/dev`, every block device included,
so the vendor partition's closed blobs are trusted as much as the kernel, and
nothing here plays the part of the PC build's verity `/usr`. It drops
`sys_module`, `sys_rawio`, `sys_time` and the MAC capabilities; a `/dev`
holding only the HAL nodes would narrow it further, but which nodes those are
is per device, and it waits for a device bring-up to find out.

`nix build .#packages.aarch64-linux.halium` produces `boot.img` and `rootfs.img.xz` with
`SHA256SUMS`. Install with `fastboot flash boot boot.img`, unpack
`rootfs.img.xz`, and copy it in from a recovery with
`adb push rootfs.img /data/losos/rootfs.img`.

## What a device port supplies

`losos-desktop-halium-aarch64` has no device in it: it carries nixpkgs'
generic kernel, warns that it boots nothing, and exists so CI evaluates and
builds the target. A port is that configuration plus:

- `losos.halium.kernelPackages`: the port's kernel, for example from
  `pkgs.linuxManualConfig` over the vendor tree and its Halium defconfig. It
  must be 5.10 or newer, systemd's minimum baseline, and evaluation fails
  otherwise. That rules out most Android 9 and 10 era ports, which run 4.x
  kernels.
- `losos.halium.mkbootimgArgs` and `losos.halium.dtb`: the device's
  `BOARD_MKBOOTIMG_ARGS` and device tree, copied from its `BoardConfig.mk`.
- `losos.halium.android.system` and `.vendor` when the partition labels
  differ (A/B slots), and `losos.halium.userdataFsType` for f2fs.
- `losos.halium.android.udevRules`: rules generated from the device's
  `ueventd.rc`.
- The Halium system image itself, built from the Halium tree for the device
  and flashed to `system`.

`nixos/pkgs/` builds `libhybris` from its upstream master and Halium's
`android-headers` (one tree serves Halium 11 to 16). A device whose vendor
HALs need older headers overrides `android-headers`.
