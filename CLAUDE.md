# CLAUDE.md

Guidance for Claude Code (claude.ai/code) working in this repository.

## What this is

`losos-desktop` is a **distribution**, not a program: a DAG of
[`pm`](https://github.com/dichhead/pm) build recipes that compiles a
systemd-native GNOME desktop OS from pinned upstream tarballs and assembles a
rootfs tarball, an initramfs and a UKI. There is no application source code
here. What there is: recipe templates, a generator, a set of gates, and the
documentation that explains why each of them is shaped the way it is.

`README.md` covers what the OS is. This file covers what the README does not:
the build commands, the generation pipeline, and the `pm` behaviours that bite
silently.

## Build & develop

```sh
cd ../pm && cargo build --release   # the pm binary; this repo never vendors it

./do check        # the gate. Generate, validate, sign, lint, smoke-build.
./do configure    # recipes/**/*.in -> out/recipes/**
./do lint         # pm explain over the whole DAG + the two local lints
./do sign         # pm sign every generated recipe, repo-local trust store
./do build [pkg]  # the real build. Needs network and hours.
./do clean
```

`./do check` is the only gate that runs anywhere: no network, no KVM, no nix,
no root beyond working user namespaces. **Run it before trusting a change.**
`./do build` needs to reach the upstream source hosts and is not a gate.

## Architecture (cross-file big picture)

**Recipes are generated, never authored directly.** The tracked truth is
`recipes/<layer>/<pkg>/build.yaml.in` plus `sources.lock`; `tools/configure`
writes `out/recipes/<pkg>/build.yaml`. Three `pm` properties force this, and
`pm` solves the same problem for itself the same way (`pm.yaml.in`):

1. A build file has **no `$srcdir`**. The only host directory mounted into the
   jail is the build file's own, at its own absolute path, so every path a
   recipe names must be absolute — and therefore substituted in.
2. A `dl_urls` download lands at `/build/<digest>/<basename>`, where `<digest>`
   is **FNV-1a-64 over the full URL string, formatted `{:016x}`**
   (`Step::url_digest` in pm's `src/step.rs`). With no shell and no globbing, a
   recipe cannot discover that path at run time, so `tools/configure` computes
   it and substitutes `@DL_<KEY>@`.
3. Helper scripts must sit **inside the recipe's own directory** to be
   readable from the jail, so `tools/configure` copies `tools/lib/*.sh` next to
   every generated `build.yaml`.

**`pm` runs from `out/`.** Dependency paths in a build file resolve against the
*process* working directory, not the build file's directory, so every
`dependencies:` entry is spelled `recipes/<pkg>/build.yaml` and `./do` always
`cd`s to `out/` first. Archives also land in the process working directory.

**Every package is two recipes: `<pkg>-src` and `<pkg>`.** `Capability::Network`
is derived per *build file*, not per step — `BuildPolicy::derive` unions the
capabilities of every step and `BuildSandbox::new` unshares the network
namespace once for the whole build. So a recipe that downloads compiles in a
jail that **shares the host network**, and pm warns about exactly that. Splitting
fetch from build is the only way to keep a compiler off the network: `<pkg>-src`
downloads and restages the verified tarball, `<pkg>` depends on it and compiles
in an empty netns. This doubles the recipe count on purpose.

**The sysroot pattern** (`tools/lib/sysroot.sh`) exists because **`pm` carries
dependencies without consuming them**. A dependency's archive is copied to
`/dest/deps/<name>-<version>.cpkg` and nothing unpacks it; there is no store, no
prefix, and nothing sets `-I`, `-L` or `PKG_CONFIG_PATH`. So each recipe unpacks
its own dependency closure — each `.cpkg` carries its own nested `deps/` — into
`/build/sysroot`, then rewrites `prefix=` in every staged `.pc` file to point at
it. That rewrite is why meson, cmake and autotools all work with nothing but
native flags and no `env` wrapper.

**Layers**, bottom to top: `00-base` (the libraries systemd's maximal option set
needs) -> `10-systemd` (systemd itself, and the kernel) -> `20-graphics` ->
`30-gnome` -> `90-image` (merge, initramfs, UKI, rootfs tar). `30-gnome` is the
long tail and is deliberately last, so the tree is useful before it is complete.

**`os/`** is the part of the OS that is not upstream: unit files, drop-ins,
presets, `sysusers.d`, `tmpfiles.d`, `repart.d`, `sysupdate.d`, networkd config
and the kernel command line. `90-image` stages it verbatim.

## Gotchas that bite silently

- **`pm` verifies a detached `.sig` before it parses a build file**, and holds
  every dependency to the same standard all the way down. **Any edit
  invalidates it, including a comment.** `./do check` re-signs the whole tree
  every run for this reason; if you edit a recipe and skip signing, the failure
  reads as a trust error, not an edit.
- **The fingerprint check reads the first word only.** The pattern is anchored
  `^(?:[\w.+/-]*/)?(?:…)(?:\s|$)`, so `env FOO=bar meson setup …` matches the
  `coreutils` fingerprint and the wrapped program is never checked. That is a
  hole in the guarantee `pm explain` appears to give. `tools/lint` closes it on
  our side by re-applying pm's table past any leading `VAR=value` tokens — do
  not remove that check, and prefer a native flag over `env` every time.
- **`basename` is not in the fingerprint table**, deliberately (pm's
  `tests/cli_build.rs` asserts it). Neither is `mkfs`, `losetup`, `dd`,
  `mount`, `rpm`, `dpkg`, `mkosi` or `dracut`. If an assembly step feels like it
  wants one of those, it belongs in a `/bin/sh` helper script — or it does not
  belong inside a `pm` jail at all.
- **`python` does not grant network, but `pip`, `cargo`, `go`, `node`, `npm`
  and `git` all do.** Invoking any of the latter in a build recipe silently
  turns its jail into a network-sharing one and defeats the `-src` split. Use
  `python` for `ukify` and friends; never `pip`.
- **`/root` and `/var` are not mounted in the jail at all, and `/tmp` is a
  fresh tmpfs.** A recipe that stages into any of them writes to nowhere. The
  only writable mounts are `/build` (cwd) and `/dest` (`DESTDIR`).
- **The jail's `PATH` is fixed** to
  `/usr/local/bin:/usr/local/sbin:/usr/bin:/usr/sbin:/bin:/sbin`. `pm` resolves
  a step's *first word* against the host `PATH` and hands `execve` the
  canonicalised result, but nothing after that — so a build system's own lookup
  of `cc` or `rustc` goes through the container `PATH` and finds nothing if the
  toolchain lives outside `/usr`. Name such binaries explicitly, the way pm's
  own `pm.yaml.in` passes `--config target.<triple>.linker=`.
- **`version:` is a list of strings.** Quote every component. An unquoted `0`
  reaches serde as an integer and parsing fails with a type error that does not
  mention quoting.
- **`stage:` is required on every step.** serde does not apply the Rust
  default, so omitting it is an error rather than a `Prepare`.
- **`TMPDIR` must be on real disk.** Every build workspace lives under it, and
  a large package needs several GB. On a tmpfs `/tmp` the build dies partway
  through with `Disk quota exceeded (os error 122)`. `./do` points it at
  `out/tmp`.
- **`sources.lock` may ship unresolved hashes.** `TODO` is a sentinel, not a
  placeholder to be filled in by guessing. Run `tools/fetch-hashes` on a
  networked machine; `./do lint` fails while any remain.

## Conventions

- Commits are Conventional Commits (`feat:`, `fix:`, `docs:`, `ci:`), written
  from the diff. **No attribution trailers of any kind** — no `Co-Authored-By`,
  no "Generated with", no session link, and no tool name in a code comment or
  doc header. The sibling `losos` enforces this with a commit-msg hook.
- Licence is AGPL-3.0-or-later via the blanket `REUSE.toml`. **No per-file SPDX
  headers**: a recipe's header comment is scarce space, spent on what the recipe
  does and why.
- Comment density follows `losos`: every non-obvious line carries the reason it
  is there, in prose, at the point of use. Removed things get a tombstone
  comment saying why they are not coming back.
