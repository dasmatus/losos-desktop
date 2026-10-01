# losos-desktop

A systemd-native, image-based GNOME desktop operating system, on musl.

The OS is a NixOS configuration: `flake.nix` and `nixos/`. Every package comes
from nixpkgs 26.05, pinned by `flake.lock`, and is compiled from source for
`x86_64-unknown-linux-musl` or `aarch64-unknown-linux-musl`; nothing comes from
cache.nixos.org, only from this project's own cache in GHCR, served through
[`proxy/`](proxy/). [`pm`](https://github.com/dichhead/pm) ships in the image as
the system manager: what a user of the running system builds and runs software
with.

It is the desktop counterpart to [`losos`](https://codeberg.org/dasmatus/losos),
which is a deliberately headless appliance. `losos-desktop` inherits that
project's house style and none of its technical decisions.

## What "as much systemd as possible" means here

Not a checklist. The OS is systemd-native end to end, and each component is
wired into a job that the design actually needs:

| Concern | Component |
|---|---|
| Boot | `systemd-boot`, `systemd-stub` (UKI), `bless-boot`, `boot-check-no-failures` |
| Installation | the installer UKI runs `systemd-repart` with `CopyBlocks=` onto the target disk |
| Self-installation | `systemd-repart` in the initrd creates slot B, root, `/home` and swap on first boot |
| Immutable `/usr` | the Nix store on a dm-verity partition, its root hash on the UKI's command line; `systemd-sysext` / `systemd-confext` for layering |
| Updates | `systemd-sysupdate` A/B, and `fwupd` for firmware |
| Factory reset | `systemd-repart`'s `FactoryReset=` on root and `/home`, the reset Varlink API |
| Accounts | `systemd-homed`, `systemd-userdbd`, `pam_systemd_home`, `run0` |
| Session | `systemd-logind` seats for gdm, the systemd user manager driving `graphical-session.target` |
| Network | `systemd-networkd`, `systemd-resolved`, `systemd-timesyncd` |
| Resources | `systemd-oomd`, zswap in front of a RAM-sized encrypted swap partition |

[`docs/nixos.md`](docs/nixos.md) says how each piece is built, every outside
input the build trusts, and what is not done yet.

## Building

```sh
nix build              # the disk image: dd it to a disk and boot
nix build .#installer  # the same image, booting the installer by default
nix build .#release    # what a release uploads, with SHA256SUMS
nix run .#vm           # boot the image in QEMU with UEFI firmware
nix flake check        # both architectures, losos-security's tests, and
                       # (on a builder with KVM) a VM boot test
```

`just` has short names for the same commands, and `just plugins` builds pm's
plugin components for a pm outside the image.

From an empty cache that is the whole OS from source, which is days on one
machine. With the project's cache configured (`docs/nixos.md`, "Binary
cache"), a build compiles only what changed.

## What this repository writes itself

Almost everything comes from nixpkgs. What does not:

- `nixos/`: the modules that wire nixpkgs' packages into this OS.
- `src/losos-security`: the OS half of the security report, on the system bus
  (`docs/security-report.md`).
- `src/losos-swap`: writes a RAM-sized swap definition for `systemd-repart`.
- `plugins/`: pm plugins, so pm build files can name `nix` and the image tools.
- `proxy/`: the GHCR binary cache and update proxy.
- `tools/nix-cache-push`: fills that cache from CI.

## Licence

AGPL-3.0-or-later. See `LICENSES/` and `REUSE.toml`.
