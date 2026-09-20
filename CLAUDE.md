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
user namespaces — **once `./do plugins` has been run in the clone**. That one
step is the exception and it does need the network and a `wasm32-unknown-unknown`
target, because the components are compiled rather than committed. Without them
pm loads no plugins, the image layer's `%{losos-mkosi:esp}` expands to nothing,
and `explain-all` rejects the recipe. **Run `./do check` before trusting a
change.** `./do build` needs the sources mirrored first.

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
- **A program that decides what it is from its own name cannot survive C3.**
  pm canonicalises a step's first word on the host, and canonicalising resolves
  symlinks. rustup ships `cargo`, `rustc` and `clippy` as symlinks to `rustup`,
  which reads back the name it was invoked under to know which tool to proxy —
  so `cargo build --release` arrives in the jail as `rustup`, invoked as
  `rustup`, and dies with `error: unexpected argument '--release' found` and a
  `Usage: rustup[EXE] <+toolchain>` that names a program the recipe never
  mentioned. The `Containerfile` puts links to the real `cargo`, `rustc` and
  `rustdoc` in `/usr/local/bin`, ahead of rustup's own bin, for exactly this
  reason — and that directory is not free to choose. pm hands the jail an
  absolute program path, but the step it starts gets a `PATH` of its own, fixed
  at `/usr/local/bin:/usr/local/sbin:/usr/bin:/usr/sbin:/bin:/sbin`
  (`CONTAINER_PATH` in pm's `sandbox.rs`), and nothing on the host can add to
  it; links anywhere else are invisible to everything the step spawns. `rustc`
  is one of the three because dropping the `cargo` shim drops the toolchain
  setup it did for its children: the real cargo looks `rustc` up on `PATH` once
  per crate and finds either nothing — `could not execute process rustc -vV
  (never executed)` — or, if the links sit in rustup's own bin, a shim, which
  decides the toolchain wants syncing and tries to install a component into a
  finished image. `docs/host-requirements.md` says the same for a native
  build. Anything
  else multi-call — busybox, a `*-config` symlink farm — has the same problem,
  and it only ever shows up in a real build.
- **Autotools needs `--host`, even when the architectures match.** The build
  host runs glibc and everything above `losos-00-toolchain` is compiled against
  musl, so every `configure` in this tree is a cross build. Without `--host`
  autoconf tries to *run* what it compiles, and a musl-dynamic test program
  cannot run in a jail that mirrors the host's `/lib` (C7): `configure: error:
  cannot run C compiled programs`. `--target=` on the compile line tells clang
  what to emit and tells configure nothing, which is how thirty-six recipes
  came to be written without it. `tools/gates/cross-configure.py` is what stops
  the thirty-seventh; musl and openssl are exempt there, with reasons.
- **meson spells `--host` as `--cross-file`, and needs it for the same
  reason.** A meson build with only `--native-file` is a *native* build, and a
  native build compiles a test program and then runs it — the musl-dynamic
  binary pm's jail has no interpreter for. The failure names the wrong thing:
  `Could not invoke sanity check executable: [Errno 2] No such file or
  directory: '.../sanity_check_for_c.exe'`, which reads as the compiler having
  produced nothing rather than as a missing loader — and meson writes it to
  stdout and its own log, so pm's error block says `stderr: <no output>` and a
  reader who stops there learns nothing at all. meson skips that run only
  when a cross file's `[host_machine]` section has told it the build is cross
  (`is_cross and not has_exe_wrapper()`), so `share/cross.ini.in` is where the
  toolchain now lives and `share/native.ini.in` carries the build-machine
  compiler and deliberately no flags. Every meson step names both;
  `tools/gates/cross-configure.py` checks meson and autotools together. dbus
  found this, being the first meson package this tree ever compiled — the
  other forty-seven would have found it one build at a time.
- **`losos-15-hosttools` is the inverse, and the two rules must move together.**
  gperf, flex and gettext are *run* by the layers above — the kernel's build and
  systemd's — and pm's jail cannot execute a musl-dynamic binary at all. They
  carry a `drops: [target]` exception in `manifest/toolchain.yaml` and are
  therefore native glibc builds, so `--host` would be a false claim rather than
  a missing one and they are exempt in the gate. A recipe only gets its
  exception by *naming* it: `@CFLAGS_RSP_FLEX@`, not `@CFLAGS_RSP@`. Left
  spelling the shared one, the manifest entry changes nothing and nothing says
  so. What this costs the image is in `docs/limits.md`.
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

## Keeping the pins current

`tools/check-latest` asks every upstream what its newest stable release is.
Most of them answer with a directory listing; the thirty-two on github.com have
none, so those are asked over `api.github.com` and the answer carries the
download URL, which is why no GitHub URL is composed by string surgery. Set
`GITHUB_TOKEN` or the sweep runs out of anonymous quota a third of the way
through and reports the rest as `unknown`.

`--apply` moves three things together, and moving fewer is how the tree starts
lying about what it contains: the lock's url and version, the lock's hash back
to `TODO`, and the `version:` list of the recipe that downloads it (which
`tools/gates/versions.py` checks against the lock). `HOLD` in that file names
the sources whose newest release is not this tree's to take, with the reason --
`COMPILER_RT` is version-locked to the host clang.

Where one recipe downloads several sources, they are one upstream release
split across tarballs, and `tools/gates/versions.py` reads the version they
agree on as the recipe's. `enforce_lockstep` refuses an `--apply` that would
move some of a group and not the rest, because the result is not a tree the
gate rejects -- it is one the gate passes over, which is worse. `--only` is
repeatable so a group can be named in one run.

`.github/workflows/update-sources.yml` runs that weekly and tests **each
candidate alone** in its own matrix leg -- fetch the new bytes, hash them, run
`./do check` -- before collecting the survivors into one pull request. A leg
proves the URL exists and the tree still lints. It compiles nothing: nothing
here validates a recipe's `-D` options against sources it does not have (see
the tombstone in `tools/gates/`), so a bump that crosses a major version still
needs a human and a release note.

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
