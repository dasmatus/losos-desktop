# CLAUDE.md

Guidance for Claude Code (claude.ai/code) working in this repository.

## What this is

`losos-desktop` is an operating system, expressed as a NixOS configuration: a
systemd-native, image-based GNOME desktop on musl. `flake.nix` and `nixos/`
are the OS; nixpkgs 26.05, pinned in `flake.lock`, provides the packages, and
every one is compiled from source. [`pm`](https://github.com/dichhead/pm)
ships in the image as the system manager.

`README.md` says what the OS is. `docs/nixos.md` is the map: how each piece is
built, every outside input the build trusts, and what is not done. Read it
before changing anything in `nixos/`.

## Build & develop

```sh
nix build              # the disk image (.#image)
nix build .#release    # what a release uploads, with SHA256SUMS
nix flake check        # both architectures, losos-security's tests, VM boot (KVM)
nix fmt                # nixfmt; CI runs `nix fmt -- --ci`
just plugins           # pm's plugin components, for a pm outside the image
```

**Evaluate before trusting a change:** `nix eval
.#nixosConfigurations.losos-desktop-{x86_64,aarch64}.config.system.build.toplevel.drvPath`
runs every module and assertion for both architectures in a few minutes. A full
build from an empty cache is the whole OS from source -- days, not hours --
so do not start one to check a module edit.

## Layout

- `nixos/modules/`: one file per concern (`boot`, `disk`, `update`,
  `accounts`, `desktop`, `services`, `hardware`, `installer`, `musl`, `pm`, ...);
  `default.nix` imports them all.
- `nixos/pkgs/`: the overlay. Only what nixpkgs does not have: pm, its plugins,
  `losos-security`, `losos-swap`. `patches/` holds the two gnome-control-center
  patches, which target 51.0 and are not applied (`docs/nixos.md`).
- `src/`: the two programs this repository writes, built by the overlay.
- `plugins/`: pm plugins (Rust, compiled to WebAssembly components). pm only
  consults a plugin about a command no built-in fingerprint matched.
- `proxy/`: a Vercel edge function around a Rust-to-WebAssembly core, serving
  the Nix binary cache and sysupdate files from GHCR.
- `tools/nix-cache-push`: pushes built store paths into that cache.

## Gotchas that bite silently

- **The host platform is musl.** `nixpkgs.hostPlatform` is `musl64` or
  `aarch64-multiplatform-musl`, so a package that assumes glibc fails at build
  time on the builder, not at evaluation. musl has no NSS: a systemd-homed
  user is invisible to `getpwnam()`, so GDM cannot see one until an nscd
  forwarder backed by `io.systemd.UserDatabase` exists (`docs/nixos.md`,
  "musl"). Anything prebuilt against glibc -- NVIDIA's userspace, for one --
  cannot load at all; `hardware.nix` takes only NVIDIA's open kernel modules.
- **cache.nixos.org is switched off, not outranked.** Its binaries are glibc
  builds this OS is not made of. A build substitutes only from the project's
  own cache, and a key it was not told to trust is refused.
- **`/usr` is the Nix store, on dm-verity.** The root hash is `usrhash=` on the
  UKI's command line, so a different `/usr` needs a different UKI, and root
  holds only state. An update is a new `/usr` from systemd-sysupdate, never a
  generation switch; `nix.enable = false` in the image is deliberate.
- **pm's jail sees `/nix/store` read-only and no daemon socket.** A pm step
  that runs nix uses `nix --store /build/nix ...`, and `--offline` is what the
  `losos-nix` plugin keys a network-free step on. pm's grant is per build file,
  so one networked step puts the whole file on the host network.
- **`writeShellApplication` lints with ShellCheck, which is Haskell.** On musl
  that is a GHC bootstrap; `musl.nix` turns GHC off, so a new shell wrapper
  should not reintroduce it.

## What was here, and is not coming back

Until this tree switched to nixpkgs it was also a from-source distribution
built by pm: ninety-odd recipes under `recipes/`, composed into layer bundles
by Python under `tools/`, against a sysroot assembled by `share/`, with OS
content in `overlay/`, a `Containerfile` for the build host and a set of gates
that re-implemented pm's fingerprint table. nixpkgs builds every one of those
packages, and the flake already wired them together, so keeping a second
build of the same OS meant maintaining two of everything. What it still
did better than nixpkgs -- cross-DSO CFI and ThinLTO across the whole tree --
went with it; `docs/nixos.md` says so. `git log` before the removal has all of
it, including `docs/pm-constraints.md`, the C1-C11 every comment in it cited.

## CI

`.github/workflows/ci.yml` is the one workflow, plain on purpose. `flake`
builds `.#release` from source per architecture within a time budget and
pushes whatever finished to the GHCR cache, so each run continues from where
the last stopped; once a build completes it checks formatting and the flake
and, outside a pull request, pushes the release to GHCR. `publish` replaces
the `nightly` GitHub release with what reached GHCR and moves the
`images:nightly-<arch>` tag `proxy/` serves updates from. `proxy` tests the
proxy and deploys nothing. `ci` is the aggregate check.

## Conventions

- Commits are Conventional Commits (`feat:`, `fix:`, `docs:`), written from the
  diff. **No attribution trailers of any kind** — no `Co-Authored-By`, no
  "Generated with", no session link, no tool name in a comment or doc header.
  The sibling `losos` enforces this with a commit-msg hook.
- Licence is AGPL-3.0-or-later via the blanket `REUSE.toml`. **No per-file SPDX
  headers**: a module's header comment is scarce space, spent on what it does
  and why.
- Comment density follows `losos`: every non-obvious line carries the reason it
  is there, in prose, at the point of use. Removed things get a tombstone
  comment saying why they are not coming back.
