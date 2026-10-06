# glibc and stock packages

`flake.nix` selects nixpkgs' standard `x86_64-linux` and `aarch64-linux`
platforms. The NixOS configuration uses nixpkgs' stock glibc packages; its
only overlay adds this repository's own packages and patches GTK and Qt
([GTK and Qt on a phone](gtk-qt-phone.md)). In particular, ShellCheck
and the package-generated documentation retain nixpkgs' defaults. glibc NSS
lets GDM and ordinary account lookups see systemd-homed users through
`nss-systemd` and the configured nscd forwarder.
