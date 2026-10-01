# CLAUDE.md

Guidance for Claude Code (claude.ai/code) working in this repository.

## What this is

`losos-desktop` is a GNOME desktop OS written as a NixOS configuration. It
installs as an image, uses systemd for everything it can, and runs on musl.
`flake.nix` and `nixos/` are the OS. nixpkgs 26.05, pinned in `flake.lock`,
provides the packages, and the build compiles every one from source.
[`pm`](https://github.com/dichhead/pm) ships in the image as the system
manager.

`README.md` says what the OS is. `docs/nixos.md` says how each piece is built,
which outside inputs the build trusts, and what is not done. Read it before
changing anything in `nixos/`.

## Build & develop

```sh
nix build              # the disk image (.#image)
nix build .#release    # what a release uploads, with SHA256SUMS
nix flake check        # both architectures, losos-security's tests, VM boot with KVM
nix fmt                # nixfmt; CI runs `nix fmt -- --ci`
just plugins           # pm's plugin components, for a pm outside the image
```

Evaluate both architectures before trusting a change:

```sh
nix eval --raw .#nixosConfigurations.losos-desktop-x86_64.config.system.build.toplevel.drvPath
nix eval --raw .#nixosConfigurations.losos-desktop-aarch64.config.system.build.toplevel.drvPath
```

That runs every module and assertion in a few minutes. A full build from an
empty cache compiles the whole OS and takes days, so don't start one to check
a module edit.

## Layout

- `nixos/modules/` has one file per concern: `boot`, `disk`, `update`,
  `accounts`, `desktop`, `services`, `hardware`, `installer`, `musl`, `pm` and
  a few more. `default.nix` imports them all.
- `nixos/pkgs/` is the overlay, and holds only what nixpkgs lacks: pm, its
  plugins, `losos-security`, `losos-swap`, and the pm payload builder.
  `patches/` holds the two gnome-control-center patches. They target 51.0 and
  nothing applies them (`docs/nixos.md`).
- `src/` holds the two programs this repository writes. The overlay builds
  them.
- `plugins/` holds pm plugins, written in Rust and compiled to WebAssembly
  components. pm asks a plugin about a command only when no built-in
  fingerprint matched it.
- `proxy/` is a Vercel edge function with its logic in Rust compiled to
  WebAssembly. It serves the Nix binary cache and sysupdate's files from GHCR.
- `tools/nix-cache-push` pushes built store paths into that cache.

## Gotchas that bite silently

- **The host platform is musl.** `nixpkgs.hostPlatform` is `musl64` or
  `aarch64-multiplatform-musl`. A package that assumes glibc evaluates fine
  and then fails on the builder. musl has no NSS, so `getpwnam()` cannot see a
  systemd-homed user, and GDM won't list one until someone writes an nscd
  forwarder that answers from `io.systemd.UserDatabase` (`docs/nixos.md`,
  "musl"). Nothing prebuilt against glibc loads at all, NVIDIA's userspace
  included, which is why `hardware.nix` takes only the open kernel modules.
- **cache.nixos.org is off.** Its binaries are glibc builds. A build
  substitutes only from the project's own cache and refuses any path not
  signed by the key it was given.
- **`/usr` is the Nix store, on dm-verity.** The root hash is `usrhash=` on the
  UKI's command line, so a different `/usr` needs a different UKI, and root
  holds only state. An update is a new `/usr` from systemd-sysupdate. Nothing
  ever switches a generation, which is why the image sets `nix.enable = false`.
- **pm's jail mounts `/nix/store` read-only and has no daemon socket.** A pm
  step that runs nix uses `nix --store /build/nix ...`. The `losos-nix` plugin
  denies network to a step that passes `--offline`. pm grants network per
  build file, so one networked step puts the whole file on the host network.
- **`writeShellApplication` runs ShellCheck, which is written in Haskell.** On
  musl that means bootstrapping GHC, which `musl.nix` turns off. Don't add a
  shell wrapper that brings it back.

## What was here, and is not coming back

This repository used to build the same OS a second time, as a from-source
distribution built by pm. It had ninety-odd recipes under `recipes/`, Python
under `tools/` that composed them into layer bundles, a sysroot assembled by
`share/`, OS files in `overlay/`, a `Containerfile` for the build host, and
gates that re-implemented pm's fingerprint table. nixpkgs builds every one of
those packages and the flake already wired them together, so the second build
was two of everything to maintain. It did one thing nixpkgs doesn't: it
compiled the whole tree with cross-DSO CFI and ThinLTO. That went with it, and
`docs/nixos.md` says so. The history before the removal has all of it,
including `docs/pm-constraints.md` and the C1 to C11 constraints its comments
cited.

## CI

`.github/workflows/ci.yml` is the only workflow. `flake` builds `.#release`
from source for each architecture within a time budget and pushes whatever
finished to the GHCR cache, so each run picks up where the last stopped. When
a build completes it checks formatting and the flake, and outside a pull
request hands the release to `push` as an artifact. `push` uploads it to
GHCR. It is the only job holding `packages: write` beside release files, and
it checks out no code. `publish` replaces the `nightly` GitHub release and
moves the `images:nightly-<arch>` tag that `proxy/` serves updates from.
`proxy` tests the proxy and deploys nothing. `ci` is the check branch
protection reads.

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
