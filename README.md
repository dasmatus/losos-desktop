# losos-desktop

A systemd-native GNOME desktop operating system, built from source by
[`pm`](https://github.com/dichhead/pm).

This repository is a **distribution**: a signed DAG of `pm` recipes that
compiles systemd with essentially every component upstream can enable, stacks a
GNOME session on top of it, and assembles a rootfs tarball, a
systemd-in-initramfs cpio archive and a unified kernel image.

It is the desktop counterpart to [`losos`](https://codeberg.org/dasmatus/losos),
which is a deliberately headless appliance — no desktop, no graphical session,
no seat management, and an explicit argument against all three. `losos-desktop`
inherits that project's house style and none of its technical decisions.

## What "as much systemd as possible" means here

Not a checklist. The OS is systemd-native end to end, and each component is
wired into a job that the design actually needs:

| Concern | Component |
|---|---|
| Boot | `systemd-boot`, `systemd-stub` (UKI), `ukify`, `kernel-install`, `bless-boot`, `boot-check-no-failures` |
| Measured boot | `systemd-pcrlock`, `systemd-measure`, `systemd-pcrextend`, `systemd-cryptenroll` |
| Self-installation | `systemd-repart` partitions and grows the disk on first boot — which is why this repo ships no disk-image tool |
| First boot | `systemd-firstboot`, `systemd-creds`, `systemd-machine-id-setup` |
| Immutable `/usr` | `systemd-veritysetup` (dm-verity), `systemd-sysext` / `systemd-confext` for layering |
| Updates | `systemd-sysupdate` A/B |
| Accounts | `systemd-homed`, `systemd-userdbd`, `pam_systemd` |
| Session | `systemd-logind` seats for gdm, the systemd user manager driving `graphical-session.target`, `systemd-xdg-autostart-generator`, `run0` |
| Network | `systemd-networkd`, `systemd-resolved`, `systemd-timesyncd`, `systemd-ssh-generator` |
| Observability | `systemd-journald` (+ `remote`, `upload`, `gatewayd`), `systemd-coredump`, `systemd-pstore` |
| Resources | `systemd-oomd` |
| Containers & VMs | `systemd-nspawn`, `systemd-vmspawn`, `systemd-machined`, `systemd-portabled`, `systemd-importd` |

`docs/systemd-inventory.md` is the full list: every component, the meson option
that enables it, the units it ships, and where this OS wires it in.

## Building

```sh
cd ../pm && cargo build --release      # the pm binary this repo drives
cd ../losos-desktop && ./do check      # generate, validate, sign, lint, smoke-build
./do build                             # the real thing; needs network and hours
```

`./do check` is the gate that runs anywhere: it needs no network, no KVM and no
root beyond working user namespaces. `./do build` needs to reach the upstream
source hosts.

Run `./do` with no arguments for the full list of subcommands.

## Honest limits

Read `docs/limits.md` before trusting anything here. The short version:

- **Builds link against the host's `/usr`.** `pm` mirrors the host's `/bin`,
  `/lib`, `/usr` and friends read-only into the build jail, and it has no
  package store, so a compiled artifact is coherent only on a host whose glibc
  matches. This is the ceiling of the current design, not an oversight.
- **`pm` does not consume dependencies.** A dependency's archive is *carried*
  into the dependent at `/dest/deps/`, and nothing unpacks it. The sysroot
  pattern in `tools/lib/sysroot.sh` is this repo's workaround, not a `pm`
  feature.
- **`sources.lock` ships unresolved hashes.** Populate it with
  `tools/fetch-hashes` on a machine that can reach the upstream hosts. Nothing
  here invents a SHA-256: `pm` verifies the hash *before* it parses a build
  file, so a wrong value presents as a compromised mirror.
- **The GNOME layer is the long tail** and is the least complete part of the
  tree by construction.

## Licence

AGPL-3.0-or-later. See `LICENSES/` and `REUSE.toml`.
