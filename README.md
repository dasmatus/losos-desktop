# losos-desktop

A GNOME desktop operating system that uses systemd for everything it can and
installs as an image.

The OS is a NixOS configuration, `flake.nix` plus the modules in `nixos/`.
nixpkgs 26.05, pinned by `flake.lock`, provides the stock glibc packages for
`x86_64-linux` and `aarch64-linux`. Nix uses cache.nixos.org for those packages;
this project's GHCR cache, served through [`proxy/`](proxy/), can also provide
paths used by CI.
[`pm`](https://github.com/dichhead/pm) ships in the image as the system
manager, the tool a user of the running system builds and runs software with.

[`losos`](https://codeberg.org/dasmatus/losos) is the headless sibling: an
appliance with no desktop, no graphical session and no seat management, on
purpose. This repository takes its house style and none of its technical
decisions.

## Which systemd component does which job

| Job | Component |
|---|---|
| Boot | `systemd-boot`, `systemd-stub` (UKI), `bless-boot`, `boot-check-no-failures` |
| Installation | the installer UKI runs `systemd-repart` with `CopyBlocks=` onto the target disk |
| First boot | `systemd-repart` in the initrd creates slot B, root, `/home` and swap |
| Read-only `/usr` | the Nix store on a dm-verity partition, with its root hash on the UKI's command line; `systemd-sysext` and `systemd-confext` add layers on top |
| Updates | `systemd-sysupdate` with A/B slots, and `fwupd` for firmware |
| Factory reset | `FactoryReset=` on root and `/home`, and systemd's reset Varlink API |
| Accounts | `systemd-homed`, `systemd-userdbd`, `pam_systemd_home`, `run0` |
| Session | `systemd-logind` seats for gdm, and the user manager driving `graphical-session.target` |
| Network | `systemd-networkd`, `systemd-resolved`, `systemd-timesyncd` |
| Memory | `systemd-oomd`, and zswap in front of a RAM-sized encrypted swap partition |

[`docs/nixos.md`](docs/nixos.md) covers how each piece is built, every outside
input the build trusts, and what is not done yet.

## Building

```sh
nix build              # the disk image; dd it to a disk and boot
nix build .#installer  # the same image, booting the installer by default
nix build .#release    # what a release uploads, with SHA256SUMS
nix run .#vm           # boot the image in QEMU with UEFI firmware
nix flake check        # both architectures, losos-security's tests, and the
                       # VM boot test on a builder with KVM
```

`just` has short names for the same commands. `just plugins` builds pm's
plugin components for a pm installed outside the image.

Nix substitutes stock nixpkgs packages from cache.nixos.org. The image and
project-specific packages still build locally unless available from the
project's cache (`docs/nixos.md`, "Binary cache").

## What this repository writes itself

nixpkgs provides the packages. This repository provides the rest:

- `nixos/`, the modules that put nixpkgs' packages together into this OS.
- `src/losos-security`, which serves the OS half of the security report on
  the system bus (`docs/security-report.md`).
- `src/losos-swap`, which writes a swap partition definition sized to the
  machine's RAM for `systemd-repart`.
- `plugins/`, pm plugins that let pm build files call `nix` and the image
  tools.
- `proxy/`, the GHCR binary cache and update proxy.
- `tools/nix-cache-push`, which CI uses to fill that cache.

## Licence

AGPL-3.0-or-later. See `LICENSES/` and `REUSE.toml`.
