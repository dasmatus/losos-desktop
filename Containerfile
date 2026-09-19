# The build host, as a file.
#
# `docs/host-requirements.md` is the list of programs a recipe invokes directly
# and this tree therefore cannot supply -- pm resolves every step's first word
# on the host and mirrors the host's `/usr` into the jail read-only (C1, C3, C7
# in `docs/pm-constraints.md`), so there is no bootstrap step that could install
# one. That list has until now been prose: a human read it, installed things,
# and found out whether they had the right versions by running a four-hour build
# and watching where it stopped.
#
# This is that list made executable, and it is the same image in CI and on a
# developer's machine. `docs/container.md` is the reasoning; what follows is
# only the pins and the reasons a particular package is here.
#
# Two inputs decide what lands in the image, and both are pinned:
#
#   * the base image, by digest, below;
#   * the Debian archive, by a snapshot.debian.org timestamp, so `apt-get
#     install` resolves against an archive that cannot move under us. This is
#     the whole reason the base is Debian: nothing else has a public,
#     timestamped archive, and without one a Containerfile pins the *names* of
#     its dependencies and not their contents.
#
# Everything outside those two is fetched by digest or checksum or is not
# fetched at all.

# Debian 13 (trixie). Chosen for snapshot.debian.org, and it carries systemd
# 257, which is comfortably past the 254 that systemd-repart's offline mode
# needs -- the mode that lets an image be built without a loop device and
# therefore inside a user namespace at all.
FROM debian:trixie-slim

# Moving this moves every package version in the image, and nothing else does.
# To take a newer archive: pick a timestamp from https://snapshot.debian.org/,
# change it here, rebuild, and run just check.
#
# It has to be at or after the day the base image above was built, which is
# the one ordering constraint in this file. `debian:trixie-slim` is a tag and
# tags move: the one pulled today already carries libc6 and perl-base from a
# point release later than 2026-09-01, and a package from the snapshot that
# depends on an exact earlier version of one of them then cannot be installed
# at all. See `--allow-downgrades` below for what makes that survivable rather
# than fatal.
ARG DEBIAN_SNAPSHOT=20260918T000000Z

# The Rust toolchain is pinned separately because it does not come from Debian:
# `rustup target add <arch>-unknown-linux-musl` is a host requirement (see
# docs/host-requirements.md) and Debian's rustc cannot satisfy it.
#
# It also has to be new enough to build pm, which is a constraint from another
# repository: pm depends on wasmtime for the plugin sandbox, and wasmtime 47
# requires 1.94.0. Too old and the failure is forty lines of
# "wasmtime-internal-<thing> requires rustc 1.94.0", which names the crate
# that noticed rather than the pin that is wrong. Raise this when pm's tree
# raises its floor; there is nothing here that can detect it.
ARG RUST_VERSION=1.94.0

# Which LLVM the toolchain is. Named in two places below -- the compiler runtime
# package and the unversioned symlinks -- and they have to agree, so it is one
# argument rather than two literals that can drift apart.
ARG LLVM_VERSION=19

# No SHELL directive: podman builds OCI images by default and ignores one with
# a warning, so anything relying on `sh -eux` would be relying on a line that
# did nothing. Every RUN below chains with `&&` instead, which fails on the
# first error under any shell.
ENV DEBIAN_FRONTEND=noninteractive

# Point apt at the snapshot, over HTTP rather than HTTPS, which is not a
# shortcut: `debian:trixie-slim` ships no ca-certificates, so apt cannot verify
# a TLS certificate, and ca-certificates itself can only be installed from the
# archive it cannot reach. Fetching it over HTTPS first is the same problem one
# step further in.
#
# Nothing is lost by it. `Signed-By:` below points apt at the Debian archive
# keyring the base image does carry, and apt refuses any index whose OpenPGP
# signature does not verify against it. Authenticity and integrity come from
# that signature; TLS would only have hidden which files were being fetched.
#
# `Check-Valid-Until: no` is required rather than lax: a snapshot's Release
# file is stamped at the moment the snapshot was taken, so an archive more than
# a week old is "expired" by apt's clock and every build of an older snapshot
# would fail with a date error rather than a missing package.
RUN printf '%s\n' \
      'Types: deb' \
      "URIs: http://snapshot.debian.org/archive/debian/${DEBIAN_SNAPSHOT}/" \
      'Suites: trixie trixie-updates' \
      'Components: main' \
      'Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg' \
      'Check-Valid-Until: no' \
      > /etc/apt/sources.list.d/debian.sources \
 && printf '%s\n' \
      'Types: deb' \
      "URIs: http://snapshot.debian.org/archive/debian-security/${DEBIAN_SNAPSHOT}/" \
      'Suites: trixie-security' \
      'Components: main' \
      'Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg' \
      'Check-Valid-Until: no' \
      > /etc/apt/sources.list.d/debian-security.sources \
 && rm -f /etc/apt/sources.list

# `--error-on=any` is the difference between one clear line and thirty
# misleading ones. Plain `apt-get update` exits 0 when an index fails to
# download -- it only warns -- so the build carried on with an empty package
# list and failed in `install` with "Unable to locate package" for every
# package at once, which reads as thirty missing packages rather than as one
# archive that was never reachable.
#
# `--allow-downgrades` is what reconciles the base image with the snapshot. The
# base is a tag, so its contents float; the snapshot does not. Where they
# disagree apt is permitted to move a package to the snapshot's version even
# when that is backwards -- which is the direction this file wants, since the
# snapshot is the thing that was pinned and the tag is the thing that drifted.
# Without it the failure is apt's solver printing two conflicting decisions
# about perl-base, which says nothing about a base image at all.
RUN apt-get update --error-on=any \
 && apt-get install -y --no-install-recommends --allow-downgrades \
      \
      `# The toolchain manifest/toolchain.yaml names by unversioned name.` \
      `# libclang-rt-*-dev is not optional: without it clang errors out on` \
      `# -fsanitize=cfi with a missing ignorelist, and the toolchain report` \
      `# cannot tell whether CFI works from a flag that never compiled.` \
      clang lld llvm libclang-rt-${LLVM_VERSION}-dev \
      \
      `# The Justfile is the repository entrypoint, and the rest below are` \
      `# named directly by recipes and all in pm's fingerprint table.` \
      just \
      make pkgconf tar xz-utils zstd cpio patch \
      \
      `# meson is vendored and run as python3 .../meson.py, so meson itself is` \
      `# deliberately absent -- but ninja and cmake are invoked as first words.` \
      ninja-build cmake \
      \
      `# python3-jinja2 cannot be vendored: fwupd's meson runs` \
      `# python3 -c 'import jinja2' and a pm step can set no PYTHONPATH.` \
      python3 python3-yaml python3-jinja2 \
      \
      `# Build systems reach for these for themselves during configure.` \
      `# rsync is the odd one: no recipe names it, but the kernel's` \
      `# headers_install copies the sanitised headers with it, so without it` \
      `# the very first step of the very first layer stops with` \
      `# "rsync: not found" and an exit 127 that reads as a broken recipe.` \
      bison flex bc gperf gettext rsync \
      \
      `# The image tooling. Before this image these were the reason the tree` \
      `# carried hand-written ext4, FAT, GPT, ISO and qcow2 writers: not that` \
      `# pm refuses them -- the losos-image plugin classifies them -- but that` \
      `# nothing guaranteed they were installed on whatever host ran the build.` \
      `# mkosi is deliberately not in this list; see the pin below.` \
      systemd systemd-boot-efi systemd-ukify \
      e2fsprogs dosfstools mtools erofs-utils squashfs-tools \
      xorriso qemu-utils \
      cryptsetup-bin sbsigntool \
      \
      `# mkosi imports pefile for the code paths that read a UKI back. This` \
      `# build sets Bootable=no and builds its UKIs with files/mkuki.py, so` \
      `# nothing here should reach those paths -- but an ImportError inside a` \
      `# tool that has already started partitioning is a worse way to find out` \
      `# than an extra package.` \
      python3-pefile \
      \
      `# rustup's installer and tools/fetch-sources both need to fetch, and` \
      `# fetch-sources exists precisely because it can be told about a CA that` \
      `# pm's own downloader cannot.` \
      ca-certificates curl git \
 && rm -rf /var/lib/apt/lists/*

# Debian installs the LLVM tools under versioned names and the unversioned
# symlinks are not guaranteed by the metapackages. manifest/toolchain.yaml names
# them unversioned (`ar: llvm-ar`), and a missing one surfaces as an LTO link
# full of undefined symbols rather than as a missing program, so they are made
# here rather than hoped for.
RUN for tool in ar nm ranlib strip objcopy objdump readelf; do \
      [ -e "/usr/bin/llvm-$tool" ] && continue; \
      ln -s "../lib/llvm-${LLVM_VERSION}/bin/llvm-$tool" "/usr/bin/llvm-$tool"; \
    done \
 && clang --version && llvm-ar --version >/dev/null && ld.lld --version

# Rust from rustup rather than Debian, for the musl targets. Both architectures
# are installed in one image so the same image builds the x86_64 and the aarch64
# matrix leg; wasm32 is here for plugins/build.sh, which compiles this
# distribution's pm plugins to components.
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --no-modify-path --profile minimal \
          --default-toolchain "$RUST_VERSION" \
 && rustup target add \
      x86_64-unknown-linux-musl \
      aarch64-unknown-linux-musl \
      wasm32-unknown-unknown \
 && chmod -R a+w "$RUSTUP_HOME" "$CARGO_HOME"

# mkosi, pinned to a commit rather than taken from the archive.
#
# This is the one dependency where the version is part of this repository's
# source. `recipes/90-image/losos-image/files/mkosi/` is written against a
# specific configuration surface, and mkosi's has moved under exactly the
# settings used here: `Format=esp` meant "a UKI wrapped in an ESP" until v26,
# where it became "an ESP, and a UKI only if one is asked for". The installer
# medium wants the second meaning -- it stages a UKI this tree already built --
# so on an older mkosi the installer step does not fail, it produces a
# different image. Debian trixie froze before v26.
#
# Everything else in this image is pinned by the snapshot, and this is pinned
# the same way: by content. A tag can be moved; the commit it points at today
# cannot, so the tag is resolved here and the result asserted.
#
# It lives under /usr because pm mirrors exactly `/bin /etc /lib /lib32 /lib64
# /sbin /usr` from the host into the build jail read-only (C7). A tool in /opt
# is a tool a recipe cannot see.
ARG MKOSI_COMMIT=4736cd836108a97772142c461c49f1ddb4172348
RUN git clone --filter=blob:none --quiet https://github.com/systemd/mkosi /usr/lib/mkosi \
 && git -C /usr/lib/mkosi checkout --quiet --detach "$MKOSI_COMMIT" \
 && test "$(git -C /usr/lib/mkosi rev-parse HEAD)" = "$MKOSI_COMMIT" \
 && rm -rf /usr/lib/mkosi/.git \
 && for entry in mkosi mkosi-initrd mkosi-addon mkosi-sandbox; do \
      ln -s "../lib/mkosi/bin/$entry" "/usr/bin/$entry"; \
    done \
 && mkosi --version

# pm's every workspace lives under TMPDIR and a real build needs several GB of
# it. The container's /tmp is a tmpfs by default on most runtimes, where a large
# package dies partway through with "Disk quota exceeded" -- which reads as a
# bug in the recipe and is not (C11). just already redirects TMPDIR into the
# repository; this is the default for anything that does not.
ENV TMPDIR=/var/tmp

# mkosi's sandbox binds /home unconditionally -- `--ro-bind /home /home`, not
# `--ro-bind-try` -- so an image without that directory cannot run mkosi at all,
# and the failure is a mount error naming a path nobody asked for. The Debian
# base has one; this asserts it rather than assuming it, because a future
# --no-install-recommends or a slimmer base could quietly remove it.
RUN test -d /home

WORKDIR /src
