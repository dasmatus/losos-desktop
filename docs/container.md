# The build host, and why it is a file

[`host-requirements.md`](host-requirements.md) lists what a machine must have
installed before `./do build` can work, and explains why the list cannot be
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

```sh
./do container build      # build the image
./do container check      # run the gate inside it
./do container            # a shell in it, with the repository mounted
```

## What is actually pinned

Two inputs decide what ends up in the image, and a Containerfile that pinned
neither would pin the *names* of its dependencies rather than their contents.

**The base image**, by tag today and by digest as soon as one is recorded.

**The Debian archive**, by a `snapshot.debian.org` timestamp. This is the
reason the base is Debian rather than the distribution this project is
otherwise closest to: nothing else has a public, timestamped archive, and
without one `apt-get install` resolves against whatever the mirror holds on the
day it runs. Moving `DEBIAN_SNAPSHOT` moves every package version in the image,
and nothing else does.

`Check-Valid-Until: no` is required rather than lax. A snapshot's `Release`
file is stamped at the moment the snapshot was taken, so anything more than a
week old is expired by apt's clock, and every build of an older snapshot would
fail with a date error rather than a missing package.

The Rust toolchain is pinned separately because it does not come from Debian.
`rustup target add <arch>-unknown-linux-musl` is a host requirement and
Debian's `rustc` cannot satisfy it, so rustup is installed and given a version.
Both architectures' musl targets are in one image, so the same image builds
both legs of the matrix, and `wasm32-unknown-unknown` is there for
`plugins/build.sh`.

## What running it has to get right

**pm's jail is a user namespace inside the container.** A default Docker
container cannot create one, and the failure is `Operation not permitted` from
inside pm's sandbox, which reads as a problem with the workspace. `tools/container`
passes `seccomp=unconfined` and `apparmor=unconfined` for that reason. Rootless
podman needs neither and is preferred when both are installed, because it also
maps the invoking user into the container, so artifacts the build writes into
the repository are not left owned by root.

**`TMPDIR` must be on real disk.** Every pm workspace lives under it and a
large package needs several gigabytes; on a tmpfs the build dies partway
through with `Disk quota exceeded`, which looks like a bug in the recipe and is
not (C11). `./do` already redirects it into `out/tmp`, which is bind-mounted,
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
ran. Two images built from two different snapshots of Debian should produce
byte-identical output, and where they do not, that is a bug worth a name.
