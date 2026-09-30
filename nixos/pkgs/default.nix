# The two programs this repository writes rather than fetches. Everything else
# the OS runs comes from nixpkgs.
final: _prev: {
  losos-security = final.callPackage ./losos-security.nix { };
  losos-swap = final.callPackage ./losos-swap.nix { };
}
