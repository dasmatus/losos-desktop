# What pm will and will not let a recipe do

Ground truth for this repository. Every design decision elsewhere cites a
numbered item here. Each was checked against `pm`'s source, and the ones marked
**measured** were checked by running `pm`, not by reading it.

## C1 — There is no shell

A command string is split on whitespace and `execve`d (`Step::execute`,
`BuildSandbox::run`). No quoting, globbing, pipes, redirection, variable
expansion or `cd`. `DESTDIR=/dest` is in the environment because `make install`
expands it internally, but nothing in a `run:` line expands anything.

Consequence: a loop over an unknown number of files is inexpressible as `run:`
commands. It goes in a script, invoked as two plain words
(`/bin/sh <abs>/foo.sh`), which is the escape hatch pm's own documentation
sanctions.

## C2 — Every command's first word must match a fingerprint

`BuildPolicy::derive` matches each command against a built-in table and aborts
the build before any step runs if one matches nothing. `pm explain` performs
exactly that check standalone and exits non-zero, which makes it a real lint.

The pattern is `^(?:[\w.+/-]*/)?(?:<alternation>)(?:\s|$)` — **anchored at the
program name, and it reads the first word only.** So:

- `evilmake` does not match `make`. Good.
- `env FOO=bar meson setup …` matches `coreutils` on `env`, and `meson` is
  never checked at all. **That is a hole**, and it is why this repo bans `env`
  outright rather than using it for the sysroot (see C7 and `docs/sysroot.md`).

Not in the table, among things an OS build reaches for reflexively: `basename`,
`mkfs`, `losetup`, `dd`, `mount`, `veritysetup`, `rpm`, `dpkg`, `mkosi`,
`dracut`, `rsync`, `gperf`, `flex`, `msgfmt`, `bpftool`, `wayland-scanner`.

## C3 — A jail-only path cannot be a step's first word

`BuildSandbox::resolve_path` rewrites absolute paths under the workspace and
the staging dir, returns relative paths verbatim, and otherwise
**canonicalises the program on the host**:

```
Step `x` wants `/build/sysroot/usr/bin/gperf`, which does not exist on this host
```

`/build` exists only inside the jail, so that fails before the jail is entered.
A relative path (`./sysroot/usr/bin/gperf`) is passed through, but it still has
to match C2, and nothing worth bootstrapping does.

**Consequence, and it is the single most shaping fact in this repo: pm cannot
put a pm-built program on `PATH`.** A build-time helper that a build system
looks up for itself — gperf, flex, msgfmt, bpftool, wayland-scanner,
glib-mkenums, g-ir-scanner — must reach it by **native flag**: meson
`--native-file` `[binaries]`, `configure VAR=/build/sysroot/...`,
`make LEX=...`. The build system execs them from inside the jail, where the
path exists, and pm's first-word resolution is never consulted.

This is also why `meson` is never a step's first word here: it is not installed
on the build host at all. It is vendored and run as
`python3 /build/src/meson/meson.py`, which matches the `python` fingerprint.

## C4 — stdin is `/dev/null`, so `cpio` is unusable — **measured**

`run_jailed` sets `.stdin(Stdio::from(devnull()))`. `cpio -o` takes its file
list on stdin and GNU cpio has no flag to read that list from a file, and C1
means there is no `find | cpio`. `cpio` is in the fingerprint table and is
nonetheless dead weight.

Consequence: the initramfs is written by a stdlib-only Python newc writer
(`tools/image/mkcpio.py`). The UKI is written the same way
(`tools/image/mkuki.py`) rather than with `ukify`, which needs `pefile`.

## C5 — Dependencies are carried, never consumed

A dependency's archive is copied to `/dest/deps/<name>-<version>.cpkg` and
nothing unpacks it. No store, no prefix, nothing sets `-I`, `-L` or
`PKG_CONFIG_PATH`. Only *direct* dependencies appear in `deps/`; the tree is
carried nested, not flattened.

Two consequences:

1. Each recipe must unpack its own dependency closure. That is
   `share/sysroot.sh`, and it is a workaround living in this repo, not a pm
   feature.
2. **Archive size multiplies with fan-in.** In a wide DAG, a package that
   twelve others depend on has its bytes copied into twelve archives. This is
   the reason the build graph here is a short chain of layer bundles rather
   than one node per package — see C6.

## C6 — There is no build cache

`Graph::resolve` dedupes within a single run, but `build_alone` creates a fresh
`Workspace` unconditionally and nothing anywhere checks whether an archive
already exists. **Every `pm build <top>` rebuilds every node in the graph.**

Combined with C5, a 90-node package DAG would rebuild the world on every
attempt *and* carry the base layer's bytes into every leaf. So the build graph
is six layer bundles in a chain. Per-package recipes still exist as authored
source; `tools/configure` composes each layer's fragments into one build file.

## C7 — The jail's shape

| path | access |
|---|---|
| `/build` | read-write; the working directory |
| `/dest` | read-write; `DESTDIR` |
| `/bin /etc /lib /lib32 /lib64 /sbin /usr` | read-only mirror of the host |
| the build file's own directory | read-only, at its own absolute path |
| the directory holding each dependency archive | read-only |
| `/dev` | minimal devfs |
| `/tmp` | fresh tmpfs, **not** the host's |
| `/root`, `/var`, `$HOME` | not mounted at all |

Environment is exactly five variables: `DESTDIR=/dest`, a fixed `PATH` of
`/usr/local/bin:/usr/local/sbin:/usr/bin:/usr/sbin:/bin:/sbin`, `HOME=/build`,
`TMPDIR=/tmp`, `LC_ALL=C`. Working directory `/build`.

Because the only host tree mounted is the build file's own directory, a shared
helper cannot be shared by reference. `tools/configure` copies `share/*.sh`
into every generated recipe directory.

## C8 — Network is per build file, not per step

`BuildPolicy::derive` unions the capabilities of every step, and
`BuildSandbox::new` unshares the network namespace once for the whole build. A
non-empty `dl_urls` anywhere, or any `cargo`/`go`/`npm`/`pip`/`git` command,
grants `Network` — and pm then warns that the jail **shares the host network
namespace**.

There is no way to download in one step and compile without a network in the
next, inside one build file. A layer bundle that fetches is networked
throughout. This repo states that rather than implying otherwise.

## C9 — Downloads land at a computed path — **measured**

A `dl_urls` entry is fetched to `/build/<digest>/<basename>`, where `<digest>`
is FNV-1a-64 over the URL string formatted `{:016x}` (`Step::url_digest`), and
`<basename>` is the URL's last non-empty path segment.

Nothing in pm's public surface promises that layout and no test in pm pins it.
C1 means a recipe cannot discover it at run time. `tools/configure` therefore
computes it, and `tools/check-digest` proves the computation right by serving a
file over loopback and asserting from **inside the jail** that pm put it where
`tools/configure` said. That gate passes today:

```
check-digest: pm put the download where configure said (/build/2cb387886594f580/probe.txt)
```

The digest is taken over `Url::as_str()`, which is rust-url's *normalised*
form. `tools/gates/url-canonical.py` rejects any source URL that is not already
in that form, so the string this repo hashes is provably the string pm hashes.

## C10 — Signing happens before parsing

`pm build` and `pm explain` verify a detached `<FILE>.sig` against
`$XDG_CONFIG_HOME/pm/trusted/` *before* reading the file, and hold every
dependency to the same standard all the way down. **Any edit invalidates it,
including a comment.** `just check` re-signs the whole generated tree every run
for that reason.

## C11 — Small sharp edges

- `version:` is a list of **strings**. An unquoted `1.23` reaches serde as a
  float and parsing fails with a type error that does not mention quoting.
- `stage:` is required on every step; serde does not apply the Rust default.
- `pm build` runs `Test` steps too, so `Test` is for cheap assertions about the
  staged tree, never for an upstream test suite.
- Steps sort by stage and keep authored order *within* a stage. A composed
  layer therefore keeps every member package's steps in one stage, or a
  member's `Install` step floats to the end of the layer and builds against a
  sysroot that does not exist yet.
- Dependency paths resolve against the **process** working directory, and
  archives land there too. This repo runs pm from `out/pkgs/`, kept disjoint
  from `out/recipes/`, so the read-only mount of a recipe's directory and the
  mount of the archive directory never nest.
- `TMPDIR` must be on real disk. Every workspace lives under it; on a tmpfs
  `/tmp` a large build dies with `Disk quota exceeded (os error 122)`.
