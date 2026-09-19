# The build host, and why it is a file

[`host-requirements.md`](host-requirements.md) lists what a machine must have
installed before `just build` can work, and explains why the list cannot be
shorter: pm resolves every step's first word on the host and mirrors the host's
`/usr` into the jail read-only (C1, C3, C7 in
[`pm-constraints.md`](pm-constraints.md)), so a program a recipe invokes
directly is part of the build definition. There is no bootstrap step that could
install one, because installing it would itself be a step whose first word had
to resolve on the host.

That list was prose. A person read it, installed some things, and found out
whether they had the right versions by running a long build and watching where
it stopped. `Containerfile` is the same list, executable, and it is the same
image in CI and on a developer's machine.

Mostly it is not invoked by hand. `./do build` checks whether the layer it is
about to build needs the image host -- whether its recipe reaches for mkosi,
xorriso, qemu-img or a `%{losos-mkosi:...}` symbol -- and whether this host
actually has the pinned one, and builds in the container when it does not.
So the image layer gets the right mkosi whether or not anyone remembered, and
the layers below it, which need none of this, stay on the host where they are
faster. `recipe_needs_image_host` and `build_in_container` in `do` are the two
halves of that.

The `gates` job in `images.yml` deliberately stays on an apt list of its own.
It is the fast one, it runs on every event, and having one job that does not
depend on this image being buildable is what tells a broken tree and a broken
Containerfile apart. The `container` job is the one that runs the gate inside
the image, so it is where a wrong Containerfile shows up as itself rather than
as something else. It is not the only job it can take down: `build` routes the
whole chain through the same image when the host has no pinned mkosi
(`build_in_container`), so a Containerfile that does not build, or that is
missing a tool a recipe reaches for, fails there too -- several layers in, with
the failure wearing the name of whichever recipe hit it first.

```sh
just container build      # build the image
just container-check      # run the gate inside it
just container            # a shell in it, with the repository mounted
```

## The base, and what is not pinned

The base is `archlinux:base`. `base` and not `base-devel`, because base-devel
carries gcc and binutils -- the GNU toolchain
[`host-requirements.md`](host-requirements.md) says is not required here, and
which a build system left alone with it will autodetect and reach for. Arch
also ships clang, lld, mold and LLVM as one unversioned set kept in step with
each other, so the image installs them by the same names
`manifest/toolchain.yaml` uses and the file needs no version argument and no
symlink fixups to make `llvm-ar` and friends exist.

**The distribution's packages are not pinned, and that is a decision rather
than a gap.** The base used to be `debian:trixie-slim` with a
`snapshot.debian.org` timestamp, and that snapshot was the entire reason the
base was Debian: it is the one public, timestamped archive, so an install
resolved against contents that could not move underneath it. Arch has no
equivalent, so two builds of this Containerfile a week apart install different
package versions. Reproducibility of the build host was traded for currency of
the toolchain, deliberately.

What that costs is bounded by what this image is, which is the last section of
this document: nothing from it ends up in `losos.qcow2`. The distribution is
compiled from `manifest/sources.lock`, pinned by SHA-256 and unaffected; what
floats is the set of host programs that did the compiling. The one floating
thing worth watching is `recipes/00-toolchain/compiler-rt`, which is
version-locked to the host clang by intent and pinned in `sources.lock` by
hand. That lock was already loose on Debian trixie -- clang 19 against
compiler-rt 18.1.8 -- and is now loose by however far Arch has run ahead. It
fails at the first CFI link in `losos-00-toolchain`, which is early and loud.

Anyone re-pinning this should do it on purpose and say so in the same breath,
not restore a snapshot URL because the absence of one looks like an oversight.

## What is still pinned

Two things, and a Containerfile that pinned neither would pin the *names* of
its dependencies rather than their contents.

**The Rust toolchain**, because it does not come from the distribution at all.
`rustup target add <arch>-unknown-linux-musl` is a host requirement and a
distribution `rustc` cannot satisfy it, so rustup is installed and given a
version. Both architectures' musl targets are in one image, so the same image
builds both legs of the matrix, and `wasm32-unknown-unknown` is there for
`plugins/build.sh`.

**mkosi**, by commit, and this one is not an optimisation — it is the only
dependency whose version is part of this repository's source.
`recipes/90-image/losos-image/files/mkosi/` is a configuration written against
a particular surface, and mkosi's has moved under exactly the settings used
there. `Format=esp` meant "a UKI wrapped in an ESP" until v26, where it became
"an ESP, and a UKI only if one is asked for"; the installer medium wants the
second, because the UKI it stages was built by `files/mkuki.py` with this
tree's own `.cmdline` and `.osrel` sections. On an older mkosi that step does
not fail — it produces a different image, which is the worst of the three
outcomes. Installing mkosi from a distribution package is the one thing in this
file that would have been pinned to a version and still been the wrong one.

Pinning it by commit rather than by tag is the same argument as everywhere
else: a tag is a name, and a name can be moved. The Containerfile resolves the
commit and asserts what it got. It carries more weight than it used to, being
the only version of anything the image still holds still.

mkosi is installed under `/usr` for a reason that is easy to get wrong. pm
mirrors exactly `/bin /etc /lib /lib32 /lib64 /sbin /usr` from the host into
the build jail, read-only (C7), so a tool in `/opt` or `/usr/local/src` is a
tool the image layer cannot see — and the failure is `mkosi: not found` from
inside a jail, which reads as a missing package.

## What running it has to get right

**pm's jail is a user namespace inside the container.** A default Docker
container cannot create one, and the failure is `Operation not permitted` from
inside pm's sandbox, which reads as a problem with the workspace. `tools/container`
passes `seccomp=unconfined` and `apparmor=unconfined` for that reason. Rootless
podman needs neither and is preferred when both are installed, because it also
maps the invoking user into the container, so artifacts the build writes into
the repository are not left owned by root.

**And `/proc` has to be unmasked, which is a different problem wearing the same
error.** Creating the namespace is only half of it; pm then mounts a fresh
procfs inside it, and the kernel refuses that in a non-initial user namespace
unless the caller can already see a *fully visible* procfs — one with nothing
mounted over any part of it. Both engines mask `/proc/kcore`, `/proc/keys` and
half a dozen others with bind mounts and remount `/proc/sys` read-only, which
is precisely what makes procfs no longer fully visible. The result is

```
hakoniwa: mount(Some("proc"), "/proc", Some("proc"), ...) => EPERM
```

and `check-digest` then reports `INCONCLUSIVE` and suggests
`kernel.apparmor_restrict_unprivileged_userns=0`, which is the right advice on
a bare host and does nothing here, because the host is not what is refusing.
`tools/container` passes `unmask=ALL` to podman and `systempaths=unconfined` to
docker; neither engine accepts the other's spelling, so it is the one option
there that has to know which is running.

**`TMPDIR` must be on real disk.** Every pm workspace lives under it and a
large package needs several gigabytes; on a tmpfs the build dies partway
through with `Disk quota exceeded`, which looks like a bug in the recipe and is
not (C11). `just` already redirects it into `out/tmp`, which is bind-mounted,
so this is only a default for anything that does not.

**The sibling `pm` checkout has to be mounted.** Two gates read pm's *source*
rather than its binary — `fingerprint-lint.py --check-table` reads
`policy.rs`, and `plugins.py` compares the vendored `plugin.wit` against pm's.
Both silently downgrade to a pass when they cannot find it, so a green check
against a missing pm proves less than it looks like.

## What it is not

It is not a base for the OS being built. Nothing from this image ends up in
`losos.qcow2`: the distribution is compiled from pinned upstream sources by pm,
into a sysroot, and the only thing the host contributes is the programs that
ran. Two images built a month apart, from whatever Arch held on each day,
should produce byte-identical output, and where they do not, that is a bug
worth a name. That property is what makes an unpinned build host affordable,
and it is also the thing that stops being checked if nobody ever compares two.
