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
# change it here, rebuild, and run ./do check.
ARG DEBIAN_SNAPSHOT=20260901T000000Z

# The Rust toolchain is pinned separately because it does not come from Debian:
# `rustup target add <arch>-unknown-linux-musl` is a host requirement (see
# docs/host-requirements.md) and Debian's rustc cannot satisfy it.
ARG RUST_VERSION=1.90.0

# Which LLVM the toolchain is. Named in two places below -- the compiler runtime
# package and the unversioned symlinks -- and they have to agree, so it is one
# argument rather than two literals that can drift apart.
ARG LLVM_VERSION=19

SHELL ["/bin/sh", "-eux", "-c"]
ENV DEBIAN_FRONTEND=noninteractive

# Point apt at the snapshot. `check-valid-until=no` is required rather than
# lax: a snapshot's Release file is stamped at the moment it was taken, so an
# archive more than a week old is "expired" by apt's clock and every build of an
# old snapshot would fail with a date error rather than a missing package.
RUN printf '%s\n' \
      'Types: deb' \
      "URIs: https://snapshot.debian.org/archive/debian/${DEBIAN_SNAPSHOT}/" \
      'Suites: trixie trixie-updates' \
      'Components: main' \
      'Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg' \
      'Check-Valid-Until: no' \
      > /etc/apt/sources.list.d/debian.sources \
 && printf '%s\n' \
      'Types: deb' \
      "URIs: https://snapshot.debian.org/archive/debian-security/${DEBIAN_SNAPSHOT}/" \
      'Suites: trixie-security' \
      'Components: main' \
      'Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg' \
      'Check-Valid-Until: no' \
      > /etc/apt/sources.list.d/debian-security.sources \
 && rm -f /etc/apt/sources.list

RUN apt-get update && apt-get install -y --no-install-recommends \
      \
      `# The toolchain manifest/toolchain.yaml names by unversioned name.` \
      `# libclang-rt-*-dev is not optional: without it clang errors out on` \
      `# -fsanitize=cfi with a missing ignorelist, and the toolchain report` \
      `# cannot tell whether CFI works from a flag that never compiled.` \
      clang lld llvm libclang-rt-${LLVM_VERSION}-dev \
      \
      `# Named directly by recipes and all in pm's fingerprint table.` \
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
      bison flex bc gperf gettext \
      \
      `# The image tooling. Before this image these were the reason the tree` \
      `# carried hand-written ext4, FAT, GPT, ISO and qcow2 writers: not that` \
      `# pm refuses them -- the losos-image plugin classifies them -- but that` \
      `# nothing guaranteed they were installed on whatever host ran the build.` \
      systemd systemd-boot-efi systemd-ukify mkosi \
      e2fsprogs dosfstools mtools erofs-utils squashfs-tools \
      xorriso qemu-utils \
      cryptsetup-bin sbsigntool \
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

# pm's every workspace lives under TMPDIR and a real build needs several GB of
# it. The container's /tmp is a tmpfs by default on most runtimes, where a large
# package dies partway through with "Disk quota exceeded" -- which reads as a
# bug in the recipe and is not (C11). ./do already redirects TMPDIR into the
# repository; this is the default for anything that does not.
ENV TMPDIR=/var/tmp

# mkosi's sandbox binds /home unconditionally -- `--ro-bind /home /home`, not
# `--ro-bind-try` -- so an image without that directory cannot run mkosi at all,
# and the failure is a mount error naming a path nobody asked for. The Debian
# base has one; this asserts it rather than assuming it, because a future
# --no-install-recommends or a slimmer base could quietly remove it.
RUN test -d /home

WORKDIR /src
