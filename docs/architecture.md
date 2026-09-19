# Architecture

How a directory of YAML templates becomes three boot artifacts, and why each
part is shaped the way it is. Every "why" here bottoms out in
[`pm-constraints.md`](pm-constraints.md), whose items are numbered C1–C11.

```
recipes/<layer>/<pkg>/build.yaml.in      authored, tracked
manifest/{layers,sources}.{yaml,lock}    authored, tracked
overlay/                                 authored, tracked
        |
        |  tools/configure  — substitute, compose, stage helpers
        v
out/recipes/<layer>/build.yaml           generated, signed, disposable
        |
        |  pm build  — one jail per layer, in a chain
        v
out/pkgs/<layer>.cpkg                    each carrying the one below it
        |
        v
losos-90-image  ->  artifacts/{losos-rootfs.tar.xz, initrd.img, losos.efi}
```

## Why recipes are generated

Three pm properties leave no alternative, and pm solves the same problem for
its own build file the same way (`pm.yaml.in`):

1. **No `$srcdir`** (C1). The only host directory mounted into the jail is the
   build file's own, at its own absolute path. Every path a recipe names must be
   absolute, and an absolute path is machine-specific.
2. **Downloads land at a computed path** (C9): `/build/<fnv1a64(url)>/<name>`.
   There is no shell in a build step, so no globbing and no command
   substitution — a recipe cannot discover that directory at run time. It has to
   be told.
3. **Shared helpers cannot be shared by reference** (C7). `share/*.sh` is copied
   into every generated recipe directory.

## Why the graph is a chain of layers

pm has no build cache and copies each dependency's *entire* archive into its
dependent (C5, C6). A ninety-node package DAG would rebuild the world on every
attempt, and a package that twelve others depend on would appear twelve times in
the archive above them.

So `manifest/layers.yaml` defines seven bundles in a chain, and
`tools/lib/compose.py` splices each layer's per-package fragments into one build
file. Per-package recipes stay the authored unit — one directory, one upstream,
reviewable alone — and are not what pm sees.

The composer's one non-obvious rule: **every member step lands in the `Build`
stage**. pm sorts steps by stage and keeps authored order only within a stage,
so a member that kept an `Install` step would float past every later member and
run against a sysroot they had not populated. Stages are pm's ordering
mechanism and cannot also be the package-level one; inside a bundle, order is
the list.

## Why a sysroot, and why the prefixes get rewritten

Dependencies are carried, not consumed (C5). Each bundle therefore unpacks its
own closure into `/build/sysroot` with `share/sysroot.sh` — each `.cpkg`
carries its own nested `deps/`, so the walk is recursive — and then rewrites
`prefix=` in every staged `.pc` file.

That rewrite is the load-bearing part. A `.pc` file installed with
`--prefix=/usr` claims `prefix=/usr`, but the files are under
`/build/sysroot/usr`. A consumer reading it unmodified is handed
`-I/usr/include`, silently finds the *host's* headers in pm's read-only `/usr`
mirror, and compiles against the wrong version with no warning at all.

The alternative was `PKG_CONFIG_SYSROOT_DIR`, which would need an `env` wrapper
on every configure line — and `env` defeats pm's fingerprint check (C2) *and*
rewrites the derived capability set. Rewriting the files keeps both intact and
works identically for meson, cmake and autotools, which matters when a third of
the tree is autotools. `docs/sysroot.md` has the full argument.

Each package installs **twice**: into `/dest`, which is shipped and keeps
`prefix=/usr` verbatim, and into `/build/sysroot`, whose copy
`share/stage-sysroot.sh` re-points for the next member of the same layer.

## Why nothing built here is on `PATH`

pm canonicalises a step's first word **on the host** (C3), so a program that
exists only inside the jail can never be a step's first word. It can still be
*executed* there — it just cannot be the thing pm resolves.

This is the single most shaping fact in the repository:

- `meson` is never a step's first word. It is vendored by `losos-00-hosttools`
  and run as `python3 /build/sysroot/usr/lib/meson/meson.py`, where `python3`
  resolves on the host and `meson.py` is an argument. It is also simply not
  installed on the build hosts this targets.
- Build-time helpers a build system looks up for itself — `gperf`, `flex`,
  `msgfmt`, `bpftool` — reach it through a meson `--native-file` `[binaries]`
  section (`recipes/10-systemd/systemd/files/native.ini`). meson execs them
  itself, from inside the jail, where the paths exist.

## Why the sources come from a loopback mirror

pm's downloader is minreq built with `https-rustls`, which compiles Mozilla's
root list in (`webpki-roots`) and reads no CA environment variable. On any host
whose egress re-terminates TLS, pm cannot fetch over HTTPS at all and dies with
`invalid peer certificate: UnknownIssuer`.

`tools/fetch-sources` fetches with a tool that *can* be told about the local CA,
into a content-addressed `out/sources/<sha256>/<name>`; `tools/serve-sources`
serves that over loopback; `tools/configure --mirror` rewrites the URL while
**keeping the hash**. Nothing is weakened: a mirror serving different bytes
fails pm's check exactly as a bad upstream would. It also makes an offline or
air-gapped build work, which the upstream URLs alone never would.

## Why the image is four files and not a disk image

`mkfs`, `losetup`, `dd` and `mount` are not in pm's fingerprint table (C2), and
`/build` and `/dest` are the only writable mounts (C7). A disk image was never
available inside the jail.

It is also not wanted. `systemd-repart` creates the ESP, root and `/home`
partitions on first boot from `overlay/usr/lib/repart.d/` and grows root to the
disk it finds, so partitioning belongs to the target rather than the builder.
What the build produces is a rootfs tarball, an initramfs and two UKIs —
and `systemd-gpt-auto-generator` means there is no `/etc/fstab` and no `root=`
to write either.

The second UKI is the installer, and it is the same image with one extra word
on its command line: `systemd-sysinstall` copies the root partition it is
running from onto the target disk. So even the installer is not a fourth thing
to build — see `docs/install.md`.

The initramfs and the UKI are written by stdlib Python (`mkcpio.py`,
`mkuki.py`) because `cpio -o` reads its file list from a stdin pm ties to
`/dev/null` (C4) and `ukify` needs a module that only `pip` could install —
which would grant the whole layer network access (C8).

## The gates

`just check` runs with no network, no KVM and no nix:

| Gate | What it proves |
|---|---|
| `tools/gates/schema.py` | Every generated build file matches pm's schema, with errors that name the cause rather than serde's type mismatch. |
| `tools/gates/url-canonical.py` | Every source URL is already in rust-url's normal form, so the string this repo hashes is provably the one pm hashes. |
| `tools/check-digest` | pm really does put a download where `tools/configure` said — proved against a real `pm build` over loopback, not against our own arithmetic. |
| `tools/gates/fingerprint-lint.py` | `env` is absent, no shell metacharacters, and pm's own table re-applied past the one permitted wrapper. |
| `tools/gates/test-image.py` | The cpio and PE writers behave, checked with an independent parser. |
| `tools/gates/explain-all` | pm itself accepts every command in every layer, and each layer's capability set is what it should be — notably, the image layer has no `Network` at all. |
