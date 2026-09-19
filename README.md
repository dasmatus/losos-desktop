# losos-desktop

A systemd-native GNOME desktop operating system, built from source by
[`pm`](https://github.com/dichhead/pm).

This repository is a **distribution**: a signed DAG of `pm` recipes that
compiles systemd with essentially every component upstream can enable, stacks a
GNOME session on top of it, and assembles a rootfs tarball, a
systemd-in-initramfs cpio archive, a unified kernel image, a bootable disk
image as QCOW2 and an installer ISO.

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
| Installation | `systemd-sysinstall`, started by the installer UKI's command line — it copies the root partition it booted from onto the target |
| Self-installation | `systemd-repart` partitions and grows the disk on first boot — the shipped images carry an ESP and a root partition and nothing else |
| First boot | `systemd-firstboot`, `systemd-creds`, `systemd-machine-id-setup` |
| Immutable `/usr` | `systemd-veritysetup` (dm-verity), `systemd-sysext` / `systemd-confext` for layering |
| Updates | `systemd-sysupdate` A/B, and `fwupd` for firmware |
| Accounts | `systemd-homed`, `systemd-userdbd`, `pam_systemd` |
| Session | `systemd-logind` seats for gdm, the systemd user manager driving `graphical-session.target`, `systemd-xdg-autostart-generator`, `run0` |
| Network | `systemd-networkd`, `systemd-resolved`, `systemd-timesyncd`, `systemd-ssh-generator` |
| Observability | `systemd-journald` (+ `remote`, `upload`, `gatewayd`), `systemd-coredump`, `systemd-pstore` |
| Resources | `systemd-oomd` |
| Containers & VMs | `systemd-nspawn`, `systemd-vmspawn`, `systemd-machined`, `systemd-portabled`, `systemd-importd` |

`docs/systemd-inventory.md` is the full list: every component, the meson option
that enables it, the units it ships, and where this OS wires it in. Its rows are
the same paths a `Test` step in the systemd recipe asserts, so the document and
the build cannot drift apart — 149 of them, and a missing one fails the build.

## Building

```sh
cd ../pm && cargo build --release   # the pm binary this repo drives
cd ../losos-desktop

just plugins            # compile pm's plugin components, once per clone
just check              # the gate: generate, sign, prove the digest, lint
just fetch -- --update  # mirror the upstream sources, pinning any TODO hash
just build              # the real build
```

`just plugins` is run once per clone and needs the network and a
`wasm32-unknown-unknown` target: the WebAssembly components pm loads are
compiled from `plugins/` rather than committed, so a fresh clone has none.
Without them pm loads no plugins, the image layer's `%{losos-mkosi:esp}`
expands to nothing, and the gate rejects the recipe rather than passing over
the hole.

Past that one step `just check` runs anywhere: no network, no KVM, no nix, no
root beyond working user namespaces. It validates every generated build file against pm's schema,
checks that every source URL is in the normal form pm hashes, re-applies pm's
fingerprint table past the one permitted wrapper, tests the initramfs and UKI
writers, proves the download-path derivation against a real `pm build`, and
runs `pm explain` over all 800-odd commands in the tree.

`just build` needs the sources mirrored first. It does **not** fetch through
pm: pm's downloader compiles Mozilla's roots in and reads no CA setting, so it
cannot fetch over HTTPS on a host that re-terminates TLS. `just fetch` mirrors
the tarballs with a tool that can, and `just build` serves them over loopback
with the SHA-256 pins unchanged.

Run `just` with no arguments for the full list. A legacy wrapper remains for compatibility.

What the build host itself has to provide — `just`, clang, python3 with `jinja2`,
cargo with the musl target, and working user namespaces among them — is in
[`docs/host-requirements.md`](docs/host-requirements.md). It is short, and it
is a list rather than a bootstrap step because pm resolves a step's first word
on the host: a tool that installs a tool would have the same problem.

## What has actually been built

Honesty matters more here than ambition, so: the environment this was developed
in cannot reach kernel.org, gnome.org, freedesktop.org or github.com/systemd, so
**no upstream package in this tree has been compiled**.

What pm *has* built end to end, in the jail: `losos-00-hosttools` (meson, from a
pinned sdist) and `losos-05-core` (`losos-release`, compiled from this
repository's own C by that meson, packaged, and run back out of its own
archive). Everything above that is validated by `just check` — which proves the
tree is executable-in-principle by pm, and proves nothing about whether each
package configures.

`manifest/sources.lock` ships `sha256: TODO` for every source that could not be
fetched. That is a sentinel, not a placeholder to fill in by guessing: pm
verifies a hash before it parses a build file, so a wrong value presents as a
compromised mirror.

## Honest limits

Read `docs/limits.md` before trusting anything here. The short version:

- **Builds link against the host's `/usr`.** `pm` mirrors the host's `/bin`,
  `/lib`, `/usr` and friends read-only into the build jail, and it has no
  package store, so a compiled artifact is coherent only on a host whose glibc
  matches. This is the ceiling of the current design, not an oversight.
- **The CFI runtime is the sharp edge of that.** `00-toolchain` builds the CFI
  runtimes against musl, but clang resolves a sanitizer runtime out of its own
  resource directory and never looks in `--sysroot`, so the copy that reaches
  the link line is the host's glibc-built one. Nothing fails; the wrong library
  is simply linked. `docs/limits.md` has the measurement and the way out.
- **CFI traps rather than diagnoses, for now.** A violation stops the process
  but does not name the call site it happened at, because the diagnosing
  runtime walks the stack and there is no unwinder for the musl target in this
  tree yet. Every scheme is still enforced, cross-DSO included. It is marked
  temporary in `manifest/toolchain.yaml` with the condition that reverses it.
- **`pm` does not consume dependencies.** A dependency's archive is *carried*
  into the dependent at `/dest/deps/`, and nothing unpacks it. The sysroot
  pattern in `tools/lib/sysroot.sh` is this repo's workaround, not a `pm`
  feature.
- **Nothing here has booted.** No VM and no EFI firmware in the environment
  this was developed in. The UKI is verified structurally — it is a PE carrying
  the six sections systemd-stub looks for — and the ISO and QCOW2 are checked
  after the fact by `assert-media.py`, which reads magic numbers and offsets
  out of the finished files rather than trusting the tools that wrote them. No
  firmware has been asked to boot any of it. `tools/vm-test disk` is the test
  that would, and it needs a machine with KVM and OVMF;
  `tools/libvirt-domain` is the same machine for looking at by hand.
- **The GNOME layer is the long tail** and is the least complete part of the
  tree by construction; several control-centre panels will be inert because
  this OS runs `systemd-networkd` rather than NetworkManager.

## Installation media

`just build` produces, with no tool outside this repository and none on the
build host:

| Artifact | What it is |
|---|---|
| `losos.iso` | the installer. One file that boots from a DVD through El Torito and from a `dd`-written USB stick through its GPT, over the same bytes |
| `losos.qcow2` | a whole disk that boots to the desktop, for QEMU and libvirt |
| `losos-root.raw.xz` | the root filesystem as a partition image: what `systemd-sysupdate` writes and what the installer copies block for block |
| `losos.efi`, `losos-installer.efi` | the two unified kernel images |
| `losos-rootfs.tar.xz` | the same tree as an archive |
| `initrd.img` | the initramfs, on its own |

There is no `mkfs`, no `losetup`, no `xorriso` and no `qemu-img` behind any of
that. None of them is in pm's fingerprint table and the build jail has no
privilege to use most of them in any case, so the formats are written directly,
by the writers in `recipes/90-image/losos-image/files/` — the same reason the
initramfs is written by `mkcpio.py` rather than by `cpio`. See
[`docs/images.md`](docs/images.md).

## Licence

AGPL-3.0-or-later. See `LICENSES/` and `REUSE.toml`.
