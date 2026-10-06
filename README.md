# losos-desktop

A [derisk](https://github.com/losos-project/derisk) desktop operating system
that uses systemd for everything it can and installs as an image. It is a
NixOS configuration, `flake.nix` plus the modules in `nixos/`, on nixpkgs'
stock glibc packages for `x86_64-linux` and `aarch64-linux`, and it also
builds for [Halium](https://halium.org) phones and tablets.

The documentation is on the
[website](https://losos-project.github.io/losos-desktop/) and in the
[wiki](https://github.com/losos-project/losos-desktop/wiki). Both are
generated from [`docs/`](docs/index.md), which is where to edit it.

## Building

Clone with `git clone --recurse-submodules`, then, with `LOSOS_PROXY_URL` set
to the update deployment's base URL:

```sh
nix build --impure              # the disk image; dd it to a disk and boot
nix build .#installer --impure  # the installer ISO
nix run .#vm --impure           # boot the image in QEMU with UEFI firmware
```

[Building](docs/building.md) has the rest.

## Licence

AGPL-3.0-or-later. See `LICENSES/` and `REUSE.toml`.
