# LosOS Desktop

LosOS Desktop is a [derisk](https://github.com/losos-project/derisk) desktop
operating system that uses systemd for everything it can, installs as an
image, updates with `systemd-sysupdate`, and makes every user a
`systemd-homed` LUKS volume.

The OS is a NixOS configuration, `flake.nix` plus the modules in `nixos/`.
nixpkgs from the `nixos-unstable` channel, pinned by `flake.lock`, provides the
stock glibc packages for `x86_64-linux` and `aarch64-linux`, and this
repository decides how those packages fit together. Nix uses cache.nixos.org
for those packages; this project's GHCR cache, served through `proxy/`, can
also provide paths used by CI ([Binary cache](binary-cache.md)).
[pm](https://github.com/losos-project/pm) ships in the image as the system
manager, the tool a user of the running system builds and runs software with
([pm on LosOS](pm.md)).

Apps the image does not carry come from [Flathub](https://flathub.org) as
Flatpaks, through the [Bazaar](https://github.com/bazaar-org/bazaar) store.
derisk's own portal backend gives them its theme, screenshots and the
wallpaper; GTK's covers the file chooser and the rest. The web browser is
[Uranium](uranium.md), Chromium under the OS's own name.

The same OS also builds as one generic system image for Android phones and
tablets that launched with Android 13 or later, over [Halium](https://halium.org)
(`nixos/halium/`): NixOS's systemd initrd as the phone's `init_boot`, the
phone's own kernel, and its vendor HALs running in an LXC container. The
[web flasher](web-flasher.md) installs it from a browser. It has not run on a
device yet; [Halium GSI](halium.md) says how it boots.

[losos](https://codeberg.org/dasmatus/losos) is the headless sibling: an
appliance with no desktop, no graphical session and no seat management, on
purpose. This repository takes its house style and none of its technical
decisions.

## What went away

This repository used to build the same OS a second time, as a from-source
distribution of ninety-odd pm recipes. [What maps to what](from-the-pm-tree.md)
says where each of its pieces went. One thing went with it and has no
replacement. The recipe tree compiled every package with cross-DSO
control-flow integrity and ThinLTO, from a toolchain it configured itself.
nixpkgs on this platform enables fortify, the stack protector, stack clash
protection, RELRO, bind-now and zeroed call-used registers, going by a
derivation's `NIX_HARDENING_ENABLE`, and no CFI.

## What this repository writes itself

nixpkgs provides the packages. This repository provides the rest:

- `nixos/`, the modules that put nixpkgs' packages together into this OS.
- `src/losos-security`, which serves the OS half of the
  [security report](security-report.md) on the system bus.
- `src/losos-swap`, which writes a swap partition definition sized to the
  machine's RAM for `systemd-repart`.
- `src/losos-installer`, the backend `derisk installer` runs on the installer
  ISO: it lists the disks, partitions the chosen one with `systemd-repart`
  and installs the channel's newest release with `systemd-sysupdate`
  ([The installer](installer.md)).
- `plugins/`, pm plugins that let pm build files call `nix` and the image
  tools.
- `proxy/`, the GHCR binary cache and update proxy.
- `tools/nix-cache-push`, which CI uses to fill that cache.

## Where to start

- [Building](building.md) the image, the installer ISO and a release.
- [Everything systemd, and the exceptions](systemd.md): which systemd
  component does which job.
- [What this trusts from outside](trust.md): every input the build takes
  from somewhere else.
- [What is not done](not-done.md): the known gaps.

LosOS Desktop is licensed AGPL-3.0-or-later.
