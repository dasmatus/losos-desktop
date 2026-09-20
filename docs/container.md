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

CI jobs run in Arch Linux containers, using Arch Linux ARM on the native
arm64 runners. The `runs-on: ubuntu-24.04` and `ubuntu-24.04-arm` labels still
select GitHub's underlying VMs; GitHub does not provide a hosted Arch runner
label. Packages and job steps run in the Arch userspace, not on Ubuntu.

The `gates` job in `images.yml` deliberately keeps its own pacman package list.
It runs on every event without depending on this repository's image being
buildable, which tells a broken tree and a broken Containerfile apart. The
`container` job still builds and checks the full Containerfile on both
architectures. `build` also uses that image when the outer job lacks the pinned
mkosi, so both jobs need a nested container runtime.

Only those two jobs run privileged, for nested Podman. Jobs that invoke pm
directly instead unconfine seccomp, AppArmor and the masked system paths so
its user namespace can mount procfs. VM verification jobs expose `/dev/kvm`
when the host provides it, retaining the software-emulation fallback otherwise.
These are container options, not attempts to change the Ubuntu host's sysctls
from inside Arch. Shell steps explicitly use bash: GitHub otherwise defaults
container jobs to `sh`, which cannot read the build pipeline's `PIPESTATUS`.

Bootstrap installs git before checkout, preserving the real Git worktree
needed by source proposals and release publishing. Rust is installed explicitly
rather than inherited from the hosted runner, with its toolchain under `/usr`
and real cargo/rustc/rustdoc links where pm's jail can reach them (C3, C7).
The pm binary and encoder caches distinguish the Arch userspace from the old
Ubuntu entries; the SHA-256-verified source mirror remains shared.

```sh
just container build      # build the image
just container-check      # run the gate inside it
just container            # a shell in it, with the repository mounted
```

## The base, and what is not pinned

The base is Arch. `base` and not `base-devel`, because base-devel carries gcc
and binutils -- the GNU toolchain
[`host-requirements.md`](host-requirements.md) says is not required here, and
which a build system left alone with it will autodetect and reach for. Arch
also ships clang, lld, mold and LLVM as one unversioned set kept in step with
each other, so the image installs them by the same names
`manifest/toolchain.yaml` uses and the file needs no version argument and no
symlink fixups to make `llvm-ar` and friends exist.

**Which Arch depends on the architecture, and that is the one ugly part of
this.** Arch upstream is an x86_64-only distribution: `archlinux:base` is a
manifest list with a single `linux/amd64` entry. The `aarch64` leg of the
build matrix runs natively on an arm64 runner and routes every layer through
this image, so it cannot pull that base at all --

```
choosing an image from manifest list docker://archlinux:base: no image found
in image index for architecture arm64, variant "v8", OS linux
```

-- which arrives before a single package is installed. ARM is
[Arch Linux ARM](https://archlinuxarm.org/), a separate project with its own
build farm, and it publishes rootfs tarballs rather than container images, so
the aarch64 base is a third-party rebuild of that tree.

`tools/container` picks from `BASE_IMAGES`, keyed on the *host's*
architecture -- the machine the programs in this image have to run on, which
is a different question from `tools/configure --arch`. The Containerfile's
`BASE_IMAGE` default is the x86_64 answer, so a plain
`docker build -f Containerfile .` still works on a developer's machine, and
`--build-arg BASE_IMAGE=` overrides either.

The selection lives in Python rather than in the Containerfile because doing
it there needs the multi-stage `FROM base-${TARGETARCH}` trick, and that rests
on the builder pruning the stages it does not reach. BuildKit does; buildah
does not promise to, so under podman on an arm64 host it would pull the amd64
base and fail in exactly the way the arrangement exists to prevent.

What it costs is worth stating plainly: the two legs of the matrix are no
longer on the same toolchain version, since Arch Linux ARM lags Arch, and half
the matrix rests on an image maintainer who is not Arch. It is the narrower of
the two available trades -- the package list in the Containerfile is untouched
by it, because Arch Linux ARM is Arch's package tree rebuilt rather than a
distribution with names of its own.

The `container` job builds both images for that reason, one runner each,
independently of whether the source pins allow the image build to run.
Without the second leg, an image maintainer who stopped publishing -- or a
package that exists under one name on Arch and another on the rebuild -- could
escape that check and surface only when an aarch64 build reaches the fallback.
What the second leg does not do is make the two toolchains match; that is the
cost above, and it stays.

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
