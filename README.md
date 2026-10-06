# losos-desktop

A [derisk](https://github.com/dasmatus/derisk) desktop operating system that uses systemd for everything it can and
installs as an image.

The OS is a NixOS configuration, `flake.nix` plus the modules in `nixos/`.
nixpkgs from the `nixos-unstable` channel, pinned by `flake.lock`, provides the
stock glibc packages for `x86_64-linux` and `aarch64-linux`. Nix uses cache.nixos.org for those packages;
this project's GHCR cache, served through [`proxy/`](proxy/), can also provide
paths used by CI.
[`pm`](https://github.com/dichhead/pm) ships in the image as the system
manager, the tool a user of the running system builds and runs software with.

Apps the image does not carry come from [Flathub](https://flathub.org) as
Flatpaks, through the [Bazaar](https://github.com/bazaar-org/bazaar) store.
derisk's own portal backend gives them its theme, screenshots and the
wallpaper; GTK's covers the file chooser and the rest.

The same OS also builds for [Halium](https://halium.org) phones and tablets
(`nixos/halium/`): an Android boot image with NixOS's systemd initrd, and the
device's vendor HALs running in an LXC container. It has not run on a device
yet; `docs/nixos.md`, "Halium", says what a device port supplies.

[`losos`](https://codeberg.org/dasmatus/losos) is the headless sibling: an
appliance with no desktop, no graphical session and no seat management, on
purpose. This repository takes its house style and none of its technical
decisions.

## Which systemd component does which job

| Job | Component |
|---|---|
| Boot | `systemd-boot`, `systemd-stub` (UKI), `bless-boot`, `boot-check-no-failures` |
| Installation | a live ISO runs `derisk installer` as its session, with `wpa_supplicant` for Wi-Fi; its backend runs `systemd-repart` for the ESP and slot A, and `systemd-sysupdate` to fill them from the channel |
| First boot | `systemd-repart` in the initrd creates slot B, root, `/home` and swap; `derisk setup` asks for a language, keyboard, time zone, network and the first user, saved through localed, timedated and homed |
| Read-only `/usr` | the Nix store on a dm-verity partition, with its root hash on the UKI's command line; `systemd-sysext` and `systemd-confext` add layers on top |
| Updates | `systemd-sysupdate` with A/B slots, and `fwupd` for firmware |
| Factory reset | `FactoryReset=` on root and `/home`, and systemd's reset Varlink API |
| Accounts | `systemd-homed`, `systemd-userdbd`, `pam_systemd_home`, `run0` |
| Session | derisk as the display manager, with its lock screen as the login screen, `systemd-logind` seats, and the user manager driving `derisk-session.target` and `graphical-session.target` |
| Network | `systemd-networkd`, `systemd-resolved`, `systemd-timesyncd` |
| Memory | `systemd-oomd`, zswap in front of a RAM-sized encrypted swap partition, and GrapheneOS's hardened_malloc as every process's allocator on PCs |

[`docs/nixos.md`](docs/nixos.md) covers how each piece is built, every outside
input the build trusts, and what is not done yet.

## Building

Set `LOSOS_PROXY_URL` to the deployment's base URL (without a trailing slash)
before running these commands; image-producing outputs read it during impure
evaluation.

```sh
nix build --impure              # the disk image; dd it to a disk and boot
nix build .#installer --impure  # the installer ISO: Wi-Fi, a disk, sysupdate
nix build .#release --impure    # what a release uploads, with SHA256SUMS
nix run .#vm --impure           # boot the image in QEMU with UEFI firmware
nix flake check --impure        # both architectures, losos-security's and
                                # losos-installer's tests, and the VM boot test
                                # on a builder with KVM
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
- `src/losos-installer`, the backend `derisk installer` runs on the installer
  ISO: it lists the disks, partitions the chosen one with `systemd-repart`
  and installs the channel's newest release with `systemd-sysupdate`.
- `plugins/`, pm plugins that let pm build files call `nix` and the image
  tools.
- `proxy/`, the GHCR binary cache and update proxy.
- `tools/nix-cache-push`, which CI uses to fill that cache.

## Licence

AGPL-3.0-or-later. See `LICENSES/` and `REUSE.toml`.
