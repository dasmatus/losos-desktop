# CLAUDE.md

Guidance for Claude Code (claude.ai/code) working in this repository.

## What this is

`losos-desktop` is a **distribution**, not a program: a chain of
[`pm`](https://github.com/dichhead/pm) build recipes that compiles a
systemd-native GNOME desktop OS from pinned upstream sources and assembles a
rootfs tarball, an initramfs, a UKI, an installer ISO and a QCOW2 disk. There is almost no application source
here. What there is: recipe templates, a generator, a set of gates, and the
documentation explaining why each is shaped the way it is.

`README.md` says what the OS is. `docs/pm-constraints.md` is the ground truth
every design decision cites — **read it before changing anything**; its items
are numbered C1–C11 and referenced throughout the tree. This file is the map.

## Build & develop

```sh
cd ../pm && cargo build --release   # the pm binary; never vendored here

./do check        # the gate: configure, sign, prove the digest, lint
./do fetch        # mirror pinned sources into out/sources (needs network)
./do build        # the real build, top of manifest/layers.yaml
./do clean
```

`./do check` runs anywhere: no network, no KVM, no nix, no root beyond working
user namespaces. **Run it before trusting a change.** `./do build` needs the
sources mirrored first.

## Architecture (cross-file big picture)

**The build graph is a chain of layer bundles, not a package DAG.** pm has no
build cache (`Workspace::new` is unconditional, nothing checks for an existing
archive) and copies each dependency's *entire* archive into its dependent
(C5, C6). A ninety-node package DAG would rebuild the world on every attempt
and duplicate the base layer's bytes into every leaf that depends on it. So
`manifest/layers.yaml` defines a short chain, and `tools/lib/compose.py` splices
each layer's per-package fragments into one build file. Per-package recipes
remain the authored unit; they are not what pm sees.

The composer's one non-obvious rule: **every member step lands in the `Build`
stage.** pm sorts steps by stage and keeps authored order only *within* a stage,
so a member that kept an `Install` step would float past every later member and
run against a sysroot they had not populated yet. Inside a bundle, order is the
list.

**Recipes are generated, never authored directly.** `tools/configure` renders
`recipes/<layer>/<pkg>/build.yaml.in` into `out/recipes/<layer>/build.yaml`,
substituting absolute paths (there is no `$srcdir`, C1), the computed download
path for every source (C9), and the host toolchain paths (C3). pm has the same
problem with its own build file and solves it the same way, with `pm.yaml.in`.

**pm runs from `out/pkgs/`,** which is where archives land and where dependency
paths resolve from (C11). `out/recipes/` is kept disjoint from it so the
read-only mount of a recipe's directory never nests inside the mount of the
archive directory.

**The sysroot pattern** (`share/sysroot.sh`) exists because pm carries
dependencies without consuming them (C5). Each bundle unpacks its own dependency
closure into `/build/sysroot` — each `.cpkg` carries its own nested `deps/` — and
rewrites `prefix=` in every staged `.pc` file to point at it. Without that
rewrite a consumer is handed `-I/usr/include`, silently finds the *host's*
headers in pm's read-only `/usr` mirror, and compiles against the wrong version
with no warning at all. `share/stage-sysroot.sh` does the same for a package
just installed into the sysroot by an earlier member of the same layer.

**The local source mirror** (`tools/fetch-sources`, `tools/serve-sources`) is
not a convenience. pm's downloader is minreq built with `https-rustls`, which
compiles Mozilla's roots in (`webpki-roots`) and reads no CA environment
variable. On any host whose egress re-terminates TLS, pm cannot fetch over HTTPS
at all and dies with `invalid peer certificate: UnknownIssuer`. Fetching with a
tool that *can* be told about the local CA and serving the bytes over loopback
sidesteps that without weakening anything: the SHA-256 pin is unchanged, so a
mirror serving different bytes fails pm's check exactly as a bad upstream would.

**The image layer is mkosi, and the plugins are what let it run.** `mkosi`,
`xorriso` and `qemu-img` are all outside pm's fingerprint table (C2), and a
command matching nothing aborts the build before any step runs. `plugins/`
names them — pm consults a plugin only about a command no built-in fingerprint
matched, so this extends the table without weakening it — and the
`Containerfile` is what guarantees they are installed. mkosi drives
`systemd-repart` with `RepartOffline=yes`, which populates filesystems through
`mkfs`' own populate modes rather than a loop device the jail has no privilege
for (C7). `docs/images.md` is the reasoning; read it before touching the image
layer. The stdlib-only ext4, FAT, GPT, ISO and qcow2 writers this tree used to
carry are gone: the fingerprint argument for them was already answered by
`plugins/losos-image`, and the real gap — whether the tools were installed at
all — is the Containerfile's.

**`overlay/`** is the OS content this repo writes rather than fetches: units,
drop-ins, presets, `sysusers.d`, `tmpfiles.d`, `repart.d`, `sysupdate.d`,
networkd config, the kernel command line. The image layer stages it verbatim.

## Gotchas that bite silently

Beyond the numbered list in `docs/pm-constraints.md`:

- **`tar` needs `--no-same-owner`, everywhere, always.** pm's jail is a user
  namespace mapping only uid 0, so a tarball recording any other uid fails with
  `Cannot change ownership to uid N: Invalid argument` — *after* extracting
  everything, so it reads as a problem with the destination rather than with the
  archive's metadata.
- **`env` is banned** and `tools/gates/fingerprint-lint.py` rejects it by name.
  pm's fingerprint check reads the first word only, so `env FOO=bar meson …` is
  classified `coreutils` and `meson` is never checked — and `env` also rewrites
  the derived capability set, so `env FOO=bar cargo build` silently loses
  `Network`. Use a native flag (`--pkg-config-path`, `configure VAR=value`,
  `make VAR=value`, a meson `--native-file`). `share/in-dir.sh` is the single
  permitted wrapper, listed in `tools/gates/allowed-wrappers`, and the lint
  re-applies pm's table to what it wraps.
- **Never invoke `meson` as a step's first word.** pm canonicalises the first
  word *on the host* (C3), and meson is not installed on the build hosts this
  repo targets. It is vendored by `losos-00-hosttools` and invoked as
  `python3 /build/sysroot/usr/lib/meson/meson.py`, where `python3` is the first
  word. The same applies to every build-time helper a build system looks up for
  itself — gperf, flex, msgfmt, bpftool, wayland-scanner: they reach the build
  through a meson `--native-file` `[binaries]` section, never through `PATH`.
- **A compiled-in absolute path resolves into the host mirror.** pm's run jail
  extracts a package at `/pkg` *and* mirrors the host's `/usr` read-only, so a
  binary that opens `/usr/lib/os-release` gets the build host's file and reports
  the wrong distribution. `losos-release` resolves via `/proc/self/exe`
  instead; anything else with a data file should do the same.
- **`python` does not grant network, but `pip`, `cargo`, `go`, `node`, `npm` and
  `git` all do** — and the grant is per build file, not per step (C8), so one of
  them anywhere makes the whole layer's jail share the host network.
- **`./do check` re-signs everything on every run**, because generating a
  recipe invalidates its signature and pm verifies before parsing (C10). If you
  run pm by hand after editing, sign first or the failure reads as a trust
  error.
- **`./do build` re-generates before it builds, with the settings the last
  `tools/configure` was given** — read back from `out/configure.args`. The
  generated tree has an architecture, a channel and a version substituted into
  it and says so nowhere, so a re-generation with the defaults would turn an
  aarch64 tree into an x86_64 one and build it without complaint.
- **The release version is baked into the image**, as the name of the UKI on
  its ESP. That is the only place systemd-sysupdate can read a version from, so
  `tools/configure --version` is not cosmetic: at its default an image's kernel
  is unversioned, the first update installs a second one beside it and can
  never retire either.
- **`sources.lock` may ship unresolved hashes.** `TODO` is a sentinel, never a
  value to fill in by guessing. `tools/fetch-sources --update` writes what the
  bytes actually hashed to; `tools/configure` refuses to generate while any
  remain unless passed `--allow-unresolved`.

## Conventions

- Commits are Conventional Commits (`feat:`, `fix:`, `docs:`), written from the
  diff. **No attribution trailers of any kind** — no `Co-Authored-By`, no
  "Generated with", no session link, no tool name in a comment or doc header.
  The sibling `losos` enforces this with a commit-msg hook.
- Licence is AGPL-3.0-or-later via the blanket `REUSE.toml`. **No per-file SPDX
  headers**: a recipe's header comment is scarce space, spent on what the recipe
  does and why.
- Comment density follows `losos`: every non-obvious line carries the reason it
  is there, in prose, at the point of use. Removed things get a tombstone
  comment saying why they are not coming back.
