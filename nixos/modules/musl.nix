# What running on musl, built from source, changes in the configuration.
#
# flake.nix picks the libc; this file holds what follows from it. Each entry
# is a derivation the image would otherwise compile from source for nothing,
# which on a from-source build is hours, not megabytes.
{ lib, ... }:

{
  # The NixOS manual is rendered by evaluating every option with Nix itself,
  # so it pulls Nix and its test suite into the build of an image that has
  # no Nix. It also documents nixos-rebuild, which this OS does not have.
  documentation.nixos.enable = false;

  nixpkgs.overlays = [
    (final: prev: {
      # writeShellApplication lints every script it writes with ShellCheck,
      # a Haskell program, so one firewall helper script put GHC and a
      # hundred Haskell libraries in front of the image. This is the knob
      # nixpkgs itself turns on platforms with no GHC: the scripts are still
      # parsed with `sh -n`, and they are nixpkgs' own, linted upstream.
      shellcheck-minimal = prev.shellcheck-minimal // {
        compiler = prev.shellcheck-minimal.compiler // {
          bootstrapAvailable = false;
        };
      };
      # The same knob on pandoc, another Haskell program, which nuspell (under
      # WebKit's spell checker) and tpm2-tools each ask before rendering their
      # man pages with it. Nothing is lost but those man pages.
      pandoc = prev.pandoc // {
        compiler = prev.pandoc.compiler // {
          bootstrapAvailable = false;
        };
      };
    })
  ];

  # Accounts are the one place musl costs a feature, and nothing here can
  # paper over it: musl has no NSS, so nss-systemd is not built and
  # getpwnam() reads /etc/passwd, then asks an nscd socket. A systemd-homed
  # user is in neither, unless whatever answers that socket asks systemd's
  # userdb. docs/nixos.md, "musl", says what that means for logging in.
  #
  # A warning rather than a comment, so that every build says it until
  # nixpkgs grows NSS for musl or this tree grows the forwarder.
  warnings = [
    ''
      This image is built against musl, which has no NSS: systemd-homed users
      are not visible to getpwnam(), so GDM cannot log them in until an nscd
      forwarder backed by systemd's userdb answers for them. See docs/nixos.md.
    ''
  ];
}
