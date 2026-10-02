# The three programs this repository writes rather than fetches, and pm, the
# system manager. Everything else the OS runs comes from nixpkgs.
final: _prev: {
  pm = final.callPackage ./pm.nix { };
  pm-plugins = final.callPackage ./pm-plugins.nix { };
  losos-installer = final.callPackage ./losos-installer.nix { };
  losos-security = final.callPackage ./losos-security.nix { };
  losos-swap = final.callPackage ./losos-swap.nix { };
  # Not a package: every package in nixpkgs, as a pm package's contents.
  pm-payloads = import ./pm-payloads.nix { inherit (final) lib pkgsStatic runCommand; };
}
