# The three programs this repository writes rather than fetches, pm, the
# system manager, and the Halium hardware layer nixpkgs does not carry.
# Everything else the OS runs comes from nixpkgs.
final: _prev: {
  pm = final.callPackage ./pm.nix { };
  pm-plugins = final.callPackage ./pm-plugins.nix { };
  derisk = final.callPackage ./derisk.nix { };
  losos-installer = final.callPackage ./losos-installer.nix { };
  losos-security = final.callPackage ./losos-security.nix { };
  losos-swap = final.callPackage ./losos-swap.nix { };
  # Halium (nixos/modules/halium.nix): Android's HAL headers, and libhybris,
  # which loads the vendor's bionic-linked GPU and HAL libraries into glibc
  # processes.
  android-headers = final.callPackage ./android-headers.nix { };
  libhybris = final.callPackage ./libhybris.nix { };
  # The Android Translation Layer (nixos/modules/atl.nix), which nixpkgs does
  # not carry either; only in the closure when losos.android.enable is set.
  android-translation-layer = final.callPackage ./android-translation-layer.nix { };
  # Not a package: every package in nixpkgs, as a pm package's contents.
  pm-payloads = import ./pm-payloads.nix { inherit (final) lib pkgsStatic runCommand; };
}
