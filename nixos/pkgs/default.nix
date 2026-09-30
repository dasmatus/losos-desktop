# The two programs this repository writes rather than fetches, and pm, the
# system manager. Everything else the OS runs comes from nixpkgs.
final: _prev: {
  pm = final.callPackage ./pm.nix { };
  losos-security = final.callPackage ./losos-security.nix { };
  losos-swap = final.callPackage ./losos-swap.nix { };
}
