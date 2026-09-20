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
# Three inputs decide what lands in the image, and two of them are pinned:
#
#   * mkosi, by commit, below -- because its configuration surface is part of
#     this repository's source;
#   * the Rust toolchain, by version, because it does not come from the
#     distribution at all.
#
# The third -- the distribution's own packages -- is NOT pinned, and that is a
# deliberate call rather than an omission. See the tombstone below.

# Arch Linux. Rolling, and current with upstream in a way that matters here:
# this tree is clang-only by design (no gcc, no binutils, no ld.bfd; see
# docs/host-requirements.md) and Arch ships clang, lld, mold and LLVM as one
# unversioned set, kept in step with each other, rather than as a versioned
# package family a distribution froze at some point in its cycle.
#
# `base` rather than `base-devel`: base-devel pulls in gcc and binutils, which
# is exactly the GNU toolchain docs/host-requirements.md says is not required
# and which this image has no use for. A compiler nothing invokes is still a
# compiler a build system can autodetect and reach for.
#
# It is an argument rather than a literal because **Arch upstream is an
# x86_64-only distribution and publishes no aarch64 image**. `archlinux:base`
# carries one manifest, linux/amd64, so the aarch64 leg of the build matrix --
# which runs natively on an arm64 runner and routes every layer through this
# image -- cannot pull it at all:
#
#     choosing an image from manifest list docker://archlinux:base: no image
#     found in image index for architecture arm64, variant "v8", OS linux
#
# ARM is Arch Linux ARM, a separate project with its own build farm, and it
# publishes rootfs tarballs rather than images -- so the aarch64 base is a
# third-party rebuild of that tree. `tools/container` picks which one from the
# architecture it is running on; the default here is the x86_64 answer, so a
# plain `docker build -f Containerfile .` on a developer's machine still works.
#
# What this costs: the two legs of the matrix are no longer on the same
# toolchain version, because Arch Linux ARM lags Arch, and half the matrix
# rests on an image maintainer who is not Arch. Deliberate, and the narrower
# of the two options -- the package list below is unchanged by it, since Arch
# Linux ARM is Arch's package tree rebuilt rather than a distribution of its
# own. `docs/container.md` has the rest.
ARG BASE_IMAGE=docker.io/library/archlinux:base
FROM ${BASE_IMAGE}

# Tombstone: this file used to be `FROM debian:trixie-slim` with a
# `DEBIAN_SNAPSHOT=<timestamp>` argument pointing apt at snapshot.debian.org,
# and that snapshot was the only reason the base was Debian -- it is the one
# public, timestamped archive, so `apt-get install` resolved against contents
# that could not move under us. Arch has no equivalent, and this image is
# therefore **no longer reproducible across rebuilds**: two builds of this file
# a week apart install different package versions.
#
# That is not an oversight and it is not coming back by accident. It was asked
# and answered: reproducibility of the build *host* was traded for currency of
# the toolchain, knowingly. What it costs is bounded by what this image is --
# see "What it is not" in docs/container.md: nothing from this image ends up in
# losos.qcow2. The distribution is compiled from `manifest/sources.lock`, which
# is pinned by SHA-256 and unaffected; what floats is the set of host programs
# that did the compiling. Anyone re-pinning this should re-pin it on purpose
# and say so, not restore a snapshot URL because this comment looks like a gap.
#
# The one thing that floats and is worth watching: `recipes/00-toolchain/
# compiler-rt` is version-locked to the host clang by intent, and its pin in
# manifest/sources.lock does not move when Arch's clang does. That lock was
# already loose on Debian trixie (clang 19 against compiler-rt 18.1.8); it is
# now loose by however far Arch has run ahead. A mismatch surfaces at the first
# CFI link in `losos-00-toolchain`, which is early and loud, rather than
# silently.

# The Rust toolchain is pinned because it does not come from the distribution:
# `rustup target add <arch>-unknown-linux-musl` is a host requirement (see
# docs/host-requirements.md) and a distribution rustc cannot satisfy it.
#
# It also has to be new enough to build pm, which is a constraint from another
# repository: pm depends on wasmtime for the plugin sandbox, and wasmtime 47
# requires 1.94.0. Too old and the failure is forty lines of
# "wasmtime-internal-<thing> requires rustc 1.94.0", which names the crate
# that noticed rather than the pin that is wrong. Raise this when pm's tree
# raises its floor; there is nothing here that can detect it.
ARG RUST_VERSION=1.94.0

# Tombstone: an `LLVM_VERSION` argument used to be here, naming the LLVM the
# image installs. It existed only because Debian versions its LLVM package
# names and its install prefix -- `llvm-19-dev`, `/usr/lib/llvm-19/bin` -- and
# the file had to spell the same number in the package list and again in a loop
# that made the unversioned `llvm-ar`, `llvm-nm` and friends Debian does not
# guarantee. Arch installs one LLVM, unversioned, with its tools already on
# /usr/bin, so both the argument and that loop have nothing left to do.

# No SHELL directive: podman builds OCI images by default and ignores one with
# a warning, so anything relying on `sh -eux` would be relying on a line that
# did nothing. Every RUN below chains with `&&` instead, which fails on the
# first error under any shell.

# pacman 7 downloads inside a sandbox of its own: it drops to the `alpm` user
# and confines the download process with a Landlock ruleset. A container build
# is not allowed to apply one, so the whole transaction dies before a single
# database is fetched:
#
#     error: restricting filesystem access failed because the Landlock ruleset
#            could not be applied: Operation not permitted
#     error: switching to sandbox user 'alpm' failed!
#     error: failed to synchronize all databases (failed to retrieve some files)
#
# The official `archlinux` image already ships a pacman.conf with this turned
# off -- `scripts/make-rootfs.sh` in archlinux/archlinux-docker does it, with
# the comment "No kernel landlock in containerd" -- which is why the x86_64 leg
# passed and only the aarch64 one, on a third-party rebuild that does not, hit
# this. So it is done here rather than assumed of a base image: one code path,
# and on a base that already did it the edit rewrites the line to itself.
#
# The directive was renamed as pacman split the sandbox in two, so which one to
# write is decided by what the shipped pacman.conf knows about, the same way
# Arch's own script decides it. Writing the wrong name would be worse than
# useless: an unrecognised directive is a warning, so it would look applied and
# fail identically at the next step.
#
# The `[options]` block is printed afterwards because this edit is invisible if
# it silently matches nothing -- the failure then arrives at the step below,
# wearing the package list rather than naming the config.
RUN if grep -q '^#\?DisableSandboxFilesystem' /etc/pacman.conf; then \
      sed -i 's/^#\?DisableSandboxFilesystem.*/DisableSandboxFilesystem/' /etc/pacman.conf; \
    elif grep -q '^#\?DisableSandbox' /etc/pacman.conf; then \
      sed -i 's/^#\?DisableSandbox.*/DisableSandbox/' /etc/pacman.conf; \
    else \
      sed -i '/^\[options\]/a DisableSandbox' /etc/pacman.conf; \
    fi \
 && grep -E '^Disable' /etc/pacman.conf

# `-Syu` rather than `-S`, and that is not thoroughness. Arch does not support
# a partial upgrade: installing a package against an index newer than the
# installed base links it against library versions the base image does not
# carry, and the failure is a binary dying on a missing `.so` at the moment it
# is first run -- several layers into a build, wearing the name of whatever
# recipe reached for it. Upgrading and installing in one transaction is the
# only supported order.
#
# `--needed` so a package the base already carries is left alone rather than
# reinstalled, and the cache is dropped afterwards because nothing below reads
# it and it is a third of the image.
#
# The keyring needs no `pacman-key --init` here: the official archlinux image
# ships one already populated, and every package pacman installs is verified
# against it. That is where authenticity comes from, the same way the Debian
# archive keyring provided it before.
RUN pacman -Syu --noconfirm --needed \
      \
      `# The toolchain manifest/toolchain.yaml names by unversioned name --` \
      `# which is what Arch installs, so nothing here has to be relinked or` \
      `# renamed afterwards. compiler-rt is not optional: without it clang` \
      `# errors out on -fsanitize=cfi with a missing ignorelist, and the` \
      `# toolchain report cannot tell whether CFI works from a flag that` \
      `# never compiled.` \
      `#` \
      `# llvm is here for its cmake package as much as for its tools. Debian` \
      `# split that into llvm-N-dev; Arch does not split it at all.` \
      `# compiler-rt's own recipe configures standalone and calls` \
      `# find_package(LLVM); when that finds nothing it falls back to` \
      `# CompilerRTMockLLVMCMakeConfig, which includes AddLLVM.cmake from the` \
      `# LLVM *source* tree and hard-errors that LLVM_CMAKE_DIR does not` \
      `# exist. Nothing in the gates can see this: they never run a build.` \
      clang lld llvm compiler-rt \
      \
      `# mold is the linker manifest/toolchain.yaml names; lld is still above` \
      `# it because the kernel takes ld.lld from LLVM=1 and does not support` \
      `# mold. mold does LTO through the GNU linker-plugin interface, so it` \
      `# needs LLVMgold.so, which Debian split into a separate` \
      `# llvm-N-linker-tools package and Arch ships inside llvm itself. The` \
      `# assertion below is what proves that rather than assuming it.` \
      mold \
      \
      `# The Justfile is the repository entrypoint, and the rest below are` \
      `# named directly by recipes and all in pm's fingerprint table.` \
      just \
      make pkgconf tar xz zstd cpio patch \
      \
      `# perl, because openssl's whole build system is perl: Configure is` \
      `# a perl script, and the Makefile it writes drives more of them to` \
      `# generate the assembly and the error tables. Nothing on a command` \
      `# line here says perl, so it looks removable and is not. Arch's base` \
      `# metapackage does not carry it -- Debian's perl-base is Essential` \
      `# and came free, which is why no list had to name it there either.` \
      perl \
      \
      `# bzip2 is the odd one here: no recipe names it and nothing spells` \
      `# it on a command line. tar picks a decompressor from the file` \
      `# itself, and seven sources in manifest/sources.lock are .tar.bz2,` \
      `# so a host without it stops at the first of them -- pcre2 -- with` \
      `# a tar child that cannot exec a program no file in this tree` \
      `# mentions. Arch's base group already carries it, the way it` \
      `# carries systemd; naming it keeps the requirement written down` \
      `# rather than inherited from a base image that is free to drop it.` \
      bzip2 \
      \
      `# diffutils for the same reason, and it is the one Debian gave away` \
      `# free: diff is Priority: required there, so it was in the base image` \
      `# and no list had to name it. Arch's base metapackage does not carry` \
      `# it. libcap's Makefile diffs generated output against the checked-in` \
      `# copy to build capshdoc.c, and the failure arrives as a shell inside` \
      `# the jail reporting diff as not found -- a program no recipe mentions,` \
      `# several layers into a build, wearing the name of whichever package` \
      `# reached for it first.` \
      diffutils \
      \
      `# meson is vendored and run as python3 .../meson.py, so meson itself is` \
      `# deliberately absent -- but ninja and cmake are invoked as first words.` \
      ninja cmake \
      \
      `# python-jinja cannot be vendored: fwupd's meson runs` \
      `# python3 -c 'import jinja2' and a pm step can set no PYTHONPATH.` \
      `# Arch's python provides /usr/bin/python3, which is the name every` \
      `# recipe and every meson invocation in this tree uses.` \
      python python-yaml python-jinja \
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
      `#` \
      `# systemd is in the base already; naming it keeps the requirement` \
      `# written down rather than inherited. Arch carries systemd-boot's EFI` \
      `# stubs inside that package rather than splitting them out the way` \
      `# Debian's systemd-boot-efi did, so there is nothing to add for them;` \
      `# ukify is its own package here and is named. xorriso comes from` \
      `# libisoburn and qemu-img is a package of its own rather than part of` \
      `# a qemu-utils bundle -- both are the same program under a different` \
      `# package name, which is the only kind of difference in this list.` \
      systemd systemd-ukify \
      e2fsprogs dosfstools mtools erofs-utils squashfs-tools \
      libisoburn qemu-img \
      cryptsetup sbsigntools \
      \
      `# mkosi imports pefile for the code paths that read a UKI back. This` \
      `# build sets Bootable=no and builds its UKIs with files/mkuki.py, so` \
      `# nothing here should reach those paths -- but an ImportError inside a` \
      `# tool that has already started partitioning is a worse way to find out` \
      `# than an extra package.` \
      python-pefile \
      \
      `# rustup's installer and tools/fetch-sources both need to fetch, and` \
      `# fetch-sources exists precisely because it can be told about a CA that` \
      `# pm's own downloader cannot.` \
      ca-certificates curl git \
 && pacman -Scc --noconfirm

# What the package list above cannot state, asserted here, because each of
# these fails late and in someone else's name if it is missing.
#
# LLVMgold.so is asked for through the driver rather than spelled as a path.
# `clang -print-file-name=X` searches the compiler's own library directories
# and prints X back unchanged when it finds nothing, which is exactly the
# lookup clang performs when it hands mold `-plugin <path>` -- so this is the
# real question, not an approximation of it, and it stays right if Arch ever
# moves the file. Every link in this tree is an LTO link, so a missing plugin
# turns the first of them into "mold: fatal: could not open plugin file",
# naming a path inside clang's own directory and reading as a broken compiler.
RUN clang --version && llvm-ar --version >/dev/null && ld.lld --version \
 && ld.mold --version \
 && gold="$(clang -print-file-name=LLVMgold.so)" \
 && test -e "$gold" \
 `# The cmake package compiler-rt's standalone configure discovers, asked for` \
 `# the same way: llvm-config reports the directory find_package(LLVM) will` \
 `# resolve to, so this checks what the recipe actually needs rather than a` \
 `# path this distribution happens to use today. recipes/00-toolchain/` \
 `# compiler-rt gives the argument for discovery over literals at length.` \
 && test -d "$(llvm-config --cmakedir)"

# Rust from rustup rather than the distribution, for the musl targets. Both
# architectures are installed in one image so the same image builds the x86_64
# and the aarch64 matrix leg; wasm32 is here for plugins/build.sh, which
# compiles this distribution's pm plugins to components.
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=$PATH:/usr/local/cargo/bin
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --no-modify-path --profile minimal \
          --default-toolchain "$RUST_VERSION" \
 && rustup target add \
      x86_64-unknown-linux-musl \
      aarch64-unknown-linux-musl \
      wasm32-unknown-unknown \
 `# Everything from here to chmod is one fix, and it needs both halves.` \
 `#` \
 `# rustup ships cargo, rustc and rustdoc as SYMLINKS to rustup, and the` \
 `# binary works out which tool it is from the name it was invoked under.` \
 `# pm canonicalises a step first word on the host (C3), and canonicalising` \
 `# resolves the symlink, so a recipe asking for cargo reaches the jail as` \
 `# rustup holding cargo arguments:` \
 `#` \
 `#     error: unexpected argument --release found` \
 `#     Usage: rustup[EXE] <+toolchain>` \
 `#` \
 `# Links to the real binaries, ahead of rustup own bin on PATH, give the` \
 `# canonical path a cargo again.` \
 `#` \
 `# They go in /usr/local/bin rather than a directory of their own, because` \
 `# they have to satisfy two different PATH lookups and only one of them is` \
 `# this image to configure. pm canonicalises against the host PATH, and any` \
 `# directory would do for that. The step it then starts gets a PATH of its` \
 `# own -- fixed at /usr/local/bin:/usr/local/sbin:/usr/bin:/usr/sbin:/bin:` \
 `# /sbin, pm sandbox.rs CONTAINER_PATH -- and nothing in this file can add` \
 `# to it, so a directory outside that list is invisible to everything a` \
 `# step spawns. Measured by running a step that printed its own PATH.` \
 `#` \
 `# rustc is what a step spawns, and that is the half that is easy to miss.` \
 `# The rustup cargo shim sets the toolchain up for its child processes; the` \
 `# real cargo does not, so it looks rustc up on PATH once per crate. With` \
 `# the links in a directory of their own that lookup found nothing at all:` \
 `#` \
 `#     error: could not execute process rustc -vV (never executed)` \
 `#` \
 `# and with them in rustup own bin it found a shim, which decides the` \
 `# toolchain wants syncing and tries to install a component into an image` \
 `# that is finished:` \
 `#` \
 `#     error: component download failed for rust-src: could not rename` \
 `#     downloaded file ... No such file or directory` \
 `#` \
 `# Both surface as a cargo build failing on some dependency, naming neither` \
 `# rustup nor this file.` \
 `#` \
 `# rustup itself keeps its own name and its own bin, moved to the END of` \
 `# PATH so that bin cannot shadow the links again: rustup target add above` \
 `# and anyone updating this image still need it. What is given up is` \
 `# toolchain switching through these three names -- no +toolchain, no` \
 `# rust-toolchain.toml -- which an image that pins RUST_VERSION and installs` \
 `# exactly that toolchain has no use for.` \
 && for tool in cargo rustc rustdoc; do \
      ln -sf "$(rustup which "$tool")" "/usr/local/bin/$tool"; \
    done \
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
# different image.
#
# Pinning it by commit rather than by tag is the argument this file makes
# everywhere else about content over names: a tag can be moved; the commit it
# points at today cannot, so the tag is resolved here and the result asserted.
# It matters more now than it did, because it is the only thing in this image
# still pinned to a version of anything.
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
# and the failure is a mount error naming a path nobody asked for. Arch's
# filesystem package creates one; this asserts it rather than assuming it,
# because a future slimmer base could quietly stop doing so.
RUN test -d /home

WORKDIR /src
