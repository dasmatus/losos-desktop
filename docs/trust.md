# What this trusts from outside

Everything the flake builds from, besides this repository:

- **nixpkgs `nixos-unstable`**, the channel tarball from `channels.nixos.org`,
  pinned by
  `narHash` in `flake.lock`. Its expressions are read, not trusted blindly:
  a different tarball fails the hash.
- **Stock package binaries from cache.nixos.org.** nixpkgs' normal glibc
  package set is used without libc-specific package overrides, so available
  package closures can be substituted rather than rebuilt locally.
- **Upstream sources nixpkgs fetches**, each pinned by hash in nixpkgs, for
  packages that must be built locally.
- **Fixed-output inputs in the build closure.** CI preloads these from
  cache.nixos.org where available (`tools/nix-fetch-sources`) to avoid making
  a cold build depend on every upstream being available. The selection includes
  source archives and can also include pinned bootstrap tools or upstream
  prebuilt compilers; it is not source-only. Each copied flat or NAR output is
  checked against its derivation's hash, and outputs using other hash methods
  are rejected.
- **pm**, the `components/pm` submodule (`github.com/losos-project/pm`) at the
  commit this tree records (`nixos/pkgs/pm.nix`), and **crates.io**, for its,
  losos-security's, losos-installer's and losos-hardware's dependencies, each pinned by
  `Cargo.lock` and checked by hash.
- **Purism's adaptive GTK patches**, copied into
  `nixos/pkgs/patches/gtk3` and `gtk4` from PureOS's packaging
  (source.puri.sm, `Librem5/debs/gtk4` and `sebastian.krzyszkowiak/gtk`,
  branch `pureos/latest`). They are reviewed source in this repository, not
  fetched at build time.
- **Device firmware**, from nixpkgs' `linux-firmware`, pinned by hash like any
  source. It is prebuilt by the hardware vendors, and nothing can compile it.
  `hardware.nix` ships it because amdgpu and nouveau cannot start current
  GPUs without it.
- **NVIDIA's driver**, the production branch nixpkgs pins by hash, fetched
  from `download.nvidia.com`: the userspace libraries and GSP firmware are
  NVIDIA's binaries, which its licence lets the image and the binary cache
  redistribute unmodified. They are the image's only unfree package
  (`nixos/modules/nvidia.nix`), loaded only on a machine with a card the
  driver supports ([Hardware and NVIDIA](drivers.md)). The open kernel module
  is built from NVIDIA's MIT/GPL source.
- **derisk**, the `components/derisk` submodule
  (`github.com/losos-project/derisk`) at the commit this tree records
  (`nixos/pkgs/derisk.nix`), with its crates, mcsapi among them, pinned by its
  `Cargo.lock` and `cargoHash`. It is the desktop, the display manager and the
  portal backend. The `components/mcsapi` submodule is the same mcsapi commit,
  and builds `x2mcsapi`.
- **Home Manager**, `github.com/nix-community/home-manager`, pinned by
  revision and `narHash` in `flake.lock`. Only `nix flake check` reads it, to
  build `homeModules.pm` into a Home Manager generation
  (`nixos/tests/home-manager-pm.nix`); the image and the release never do.
- **Halium's generic system image** (`nixos/pkgs/halium-gsi.nix`), the
  Android 14 system the [Halium GSI](halium.md) runs the vendor's HALs
  under: UBports' build of Halium's AOSP tree, as Droidian publishes it in
  its apt repository, pinned by the hash that repository's index gives for
  the package. It is prebuilt bionic code nothing here can compile, and it
  runs as host root in the Android container, so it is trusted as much as
  the vendor blobs beside it. The phone's own kernel, vendor partitions and
  bootloader are the device maker's, and the GSI trusts them as Android does.
- **Flathub**, for apps installed after the fact. Its repo file, with the
  signing key every install is checked against, is
  `nixos/modules/flathub.flatpakrepo` in this tree; the image adds the remote
  from that copy at boot and never fetches the key. The apps themselves are
  Flathub's builds, sandboxed by Flatpak and verified against that key, and
  nothing in the image depends on one.
- **The project's binary cache** in GHCR, through `proxy/` ([Binary cache](binary-cache.md)): optional
  paths the CI job stores and signs. A client checks every narinfo against
  the public key it was told to trust, so the proxy and GHCR carry bytes but
  cannot vouch for them.
- **In CI only:** the official Nix installer from `releases.nixos.org`,
  pinned by version. Nix is what builds Nix, so this binary is the one
  bootstrap; `.#nix` is the nixpkgs Nix package for a build host after it.

## The components/ submodules

derisk, mcsapi and pm are git submodules under `components/`, and the flake
builds them from there (`self.submodules = true` in `flake.nix`, which Nix 2.27
and later honour). The commit recorded for each submodule is the pin: a clone
needs `git clone --recurse-submodules` (or `git submodule update --init`), and
`git -C components/<name> log` shows exactly what the image builds. Moving one
is `git -C components/<name> checkout <rev>`, a commit here, and a new
`cargoHash` in its `nixos/pkgs/*.nix` when its `Cargo.lock` changed. There is
no source hash to update: the submodule commit already names the tree.
derisk's own `Cargo.lock` still fetches mcsapi by git revision, so move
`components/mcsapi` to the revision that lock names.

android_translation_layer stays a `fetchgit` pin: it is an upstream fork, off
by default, and its tree is close to half a gigabyte, which the flake would
otherwise copy on every evaluation.
