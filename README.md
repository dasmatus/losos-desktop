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

What the build host itself has to provide — `just`, clang, python3 with `jinja2`
and `basedpyright`, cargo with the musl target, and working user namespaces
among them — is in
[`docs/host-requirements.md`](docs/host-requirements.md). It is short, and it
is a list rather than a bootstrap step because pm resolves a step's first word
on the host: a tool that installs a tool would have the same problem.

The build host is also published as
`ghcr.io/dasmatus/losos-desktop/build-host:latest` for amd64 and arm64.
Use `just container pull` and `just container-check` to reuse it without
rebuilding the Containerfile. See [container usage](docs/container.md) for
digest pinning, local builds and the pm checkout still required.

## Package containers

`tools/pm-oci` is a host-side pm extension for transporting built packages
through an OCI registry. Install `skopeo` alongside Python 3; no container daemon
or root is needed. Each image carries one unchanged `.cpkg` and its detached
`.cpkg.sig`, not an extracted root filesystem or a runnable container.

```sh
skopeo login ghcr.io
# Sign each archive you intend to publish with your chosen pm signing key.
../pm/target/release/pm sign out/pkgs/PACKAGE.cpkg
just packages -- publish --repository ghcr.io/OWNER/losos-desktop/packages \
  --tag nightly-20260920.1-x86_64 --arch x86_64 \
  --source-url https://github.com/OWNER/losos-desktop
just packages -- pull ghcr.io/OWNER/losos-desktop/packages/PACKAGE@sha256:DIGEST \
  --arch x86_64
```

Replace `OWNER` with the lowercase registry owner and use the complete
digest-pinned reference printed by publish for pull. Publish reads `out/pkgs`
(`--source` overrides it); the image name is the archive basename without
`.cpkg`. Pull writes the archive and signature into `out/pkgs` (`--output`
overrides it), refuses existing files and checks the requested architecture,
OCI digests, package checksum and container layout before installing either.
It never runs an image or extracts the package itself. Authentication uses
skopeo's normal login store or `REGISTRY_AUTH_FILE`; TLS verification stays on.

Successful main/tag builds publish to
`ghcr.io/<owner>/losos-desktop/packages/<archive-stem>:<channel>-<version>-<arch>`
and list immutable references in the workflow summary. PRs never publish or
receive registry write permission. These are built packages, **not** the
boot-tested image releases; the existing VM gates still control those releases.
GHCR package visibility is controlled by the repository owner.

The extension is deliberately **not** a WASM plugin or a new `pm pull`
subcommand: pm's plugin contract provides neither network nor filesystem access.
Use the restored archive with pm as usual. Pulling does not make `pm build`
cache-aware (C6), install packages into the OS, or alter pm's trusted keys.
A digest proves integrity, not publisher identity: obtain the reference and the
publisher's public signing key through a trusted channel. pm still authenticates
the detached signature before running a package. CI currently uses the
throwaway repo-local signing key; its packages are not automatically trusted by
other pm installations, and pulling must not silently confer that trust.
The public signer is carried in `.cpkg.sig`; after independently verifying it,
`pm trust <package>.cpkg.sig` is pm's explicit opt-in to trusting that signer.

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
- **Clang's builtin headers still come from the build host.** `00-toolchain`
  builds the CFI runtimes against musl and the tree points `-resource-dir` at
  them, so the runtime that reaches the link line is the one this repo built --
  that used to be the sharp edge here and it is closed. What a resource
  directory also has to carry is `stddef.h` and the rest of clang's builtin
  headers, which belong to the compiler rather than to compiler-rt, and those
  are copied out of the container's clang. The pins are one major version
  apart. `docs/limits.md` has the measurement.
- **Cross-DSO CFI has a hole where a version script is.** The runtime finds a
  library's `__cfi_check` by name in its dynamic symbol table, and a version
  script ending in `local: *` -- zlib's, systemd's, glib's -- localises it.
  Those libraries are inside the scheme as callers and outside it as callees.
  Open, and a design decision rather than a patch; `docs/limits.md` has the
  measurement and the three options.
- **The unwinder is built here, and CFI diagnoses rather than traps.** A
  violation names the file, line and callee of the indirect call that failed
  instead of raising SIGILL with nothing attached. That needs an unwinder,
  which musl has none of, so `00-toolchain` builds LLVM's libunwind with a patch,
  because libunwind configured on its own silently drops its assembly sources
  and produces an archive that cannot unwind. `docs/limits.md` has the
  measurement.
- **`pm` does not consume dependencies.** A dependency's archive is *carried*
  into the dependent at `/dest/deps/`, and nothing unpacks it. The sysroot
  pattern in `tools/lib/sysroot.sh` is this repo's workaround, not a `pm`
  feature.
- **Nothing in the image can authenticate anyone.** The only PAM service file
  in the tree is systemd's `systemd-user`; there is no `login` and no
  `gdm-password`, and `pam_systemd_home` is built and referenced by nothing.
  A user here is a LUKS volume rather than a passwd line, so the stack cannot
  be copied from another distribution. `docs/limits.md` has the detail.
- **There is no web browser.** Not in any layer. Both candidates are very
  large builds whose toolchain demands do not fit this tree's shape, and
  neither has been priced. `docs/limits.md` says what it costs the FIDO2
  support in `docs/yubikey.md`.
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
