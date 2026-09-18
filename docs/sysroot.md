# The sysroot, and why the prefixes get rewritten

pm carries dependencies without consuming them (C5 in
[`pm-constraints.md`](pm-constraints.md)): a dependency's `.cpkg` is copied to
`/dest/deps/<name>-<version>.cpkg`, and nothing unpacks it. There is no store,
no prefix, and nothing sets `-I`, `-L` or `PKG_CONFIG_PATH`. Every multi-package
build therefore has to solve dependency consumption itself.

`share/sysroot.sh` is this repository's answer:

1. recursively untar `/dest/deps/*.cpkg` into `/build/sysroot` — each archive
   carries its own nested `deps/`, so the walk is breadth-first over rounds;
2. drop pm's `metadata` member, which has no business in a sysroot;
3. rewrite `prefix=/usr` to `prefix=/build/sysroot/usr` in every staged `.pc`.

`share/stage-sysroot.sh` does step 3 again for a package a *later* member of the
same layer just installed, and also fixes `libdir=` in `.la` files.

## The decision

The third step is the one with alternatives, and it is the one that matters.

A `.pc` file installed with `--prefix=/usr` claims `prefix=/usr`. Its files are
under `/build/sysroot/usr`. A consumer reading it unmodified is handed
`-I/usr/include` and `-L/usr/lib` — and pm mirrors the **host's** `/usr` into
the jail read-only (C7), so those directories exist and are full of the build
host's headers and libraries. The build succeeds. It links against the wrong
version of a library this tree also built, and says nothing.

Four ways to avoid that, and why this one:

**Rewrite the `.pc` files (chosen).** Uniform across meson, cmake and
autotools, which matters when a third of the tree is autotools. No per-build-system
special case. No wrapper. The rewrite is confined to the sysroot copy; the
`/dest` copy that ships keeps `prefix=/usr`, which is correct on the target.

**`env PKG_CONFIG_SYSROOT_DIR=…` (rejected).** Two reasons. It defeats pm's
fingerprint check, which reads the first word only (C2) — `env FOO=bar meson …`
is classified `coreutils` and `meson` is never examined, so `pm explain` would
be reporting something other than what runs. And `env` *rewrites the derived
capability set*: `env FOO=bar cargo build` loses `Network` and the build then
fails in a way nobody diagnoses quickly. A mechanism whose failure mode is
"silently different sandbox" is not worth the convenience. The lint rejects
`env` by name.

**`PKG_CONFIG_SYSROOT_DIR` itself, even if it could be set (rejected).** It
prepends the sysroot to `-I` and `-L` but not to `-l`, and not to the
`Requires.private` resolution of `.pc` files that were not themselves
sysrooted. This tree is mixed-provenance by construction — some libraries from
our sysroot, libc from the host mirror — which is exactly the case it gets
wrong, in both directions.

**A meson cross/native-file `sys_root` (rejected).** meson-only. The kernel,
libcap, cryptsetup, PAM, elfutils and the whole autotools tail get nothing from
it.

## How each build system is pointed at it

No `env`, anywhere:

| System | How |
|---|---|
| meson | `--pkg-config-path=/build/sysroot/usr/lib/pkgconfig:…` plus `--native-file` |
| ninja | `-C <builddir>` |
| autotools | `configure … PKG_CONFIG_PATH=… CPPFLAGS=-I… LDFLAGS=-L…` — `configure` takes `VAR=value` as positional arguments |
| make | `make … install DESTDIR=/build/sysroot` — as a make argument, which beats the environment variable pm already set |
| cmake | `-DCMAKE_PREFIX_PATH=/build/sysroot/usr -DCMAKE_FIND_ROOT_PATH=/build/sysroot` |
| kernel | `make … LEX=/build/sysroot/usr/bin/flex` |

## The proof obligation

Rewriting prefixes risks `/build/sysroot` leaking into shipped artifacts —
libtool is the usual culprit and RPATH the usual channel, and the failure is
invisible until the target fails to load a library that is "missing" only
because the directory it names does not exist there.

So the image recipe runs `leak-audit.sh` over the assembled tree and fails if
the literal `/build` appears in any `.pc`, `.la`, `.cmake` or `*-config` file,
or inside any executable. A design whose failure mode only appears at boot needs
a gate that makes it appear at build time.

It is not a complete proof. It catches a leaked *path*; it cannot catch a
silent link against the host's copy of a library we also built. See
[`limits.md`](limits.md).
