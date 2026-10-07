# glibc and stock packages

`flake.nix` selects nixpkgs' standard `x86_64-linux` and `aarch64-linux`
platforms. The NixOS configuration uses nixpkgs' stock glibc packages; its
only overlay adds this repository's own packages and patches GTK and Qt
([GTK and Qt on a phone](gtk-qt-phone.md)). In particular, ShellCheck
and the package-generated documentation retain nixpkgs' defaults. glibc NSS
lets GDM and ordinary account lookups see systemd-homed users through
`nss-systemd` and the configured nscd forwarder.

Everything the overlay builds itself links with
[mold](https://github.com/rui314/mold) instead of nixpkgs' default
`ld.bfd`: derisk, pm and its plugins, `losos-installer`, `losos-security`,
`losos-swap`, `losos-adblock`, Danube, `x2mcsapi`, libhybris and the
Android Translation Layer, as
well as the patched GTK and Qt. The overlay calls them through one helper,
`callWithMold`, which gives a package nixpkgs' stdenv and a `rustPlatform`
built on it, both through `stdenvAdapters.useMoldLinker`, so the C
compiler and the `cc` rustc links with both pass `-fuse-ld=mold`. A
package added later is linked with mold by being called through it, with
no list to keep. WPE WebKit, which [Danube](danube.md) draws, is built
from nixpkgs' own package set and keeps its linker. Packages taken from nixpkgs unchanged keep its
linker, because a different stdenv for them would mean nothing
substituting from cache.nixos.org.
