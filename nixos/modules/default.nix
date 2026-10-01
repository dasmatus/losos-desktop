# LosOS Desktop as a NixOS module. Importing this is the whole OS; a host
# configuration adds nothing but its architecture and version (see flake.nix).
#
# docs/nixos.md maps each file here to the piece of the pm tree it replaces.
{
  imports = [
    ./options.nix
    ./boot.nix
    ./hardware.nix
    ./image.nix
    ./disk.nix
    ./update.nix
    ./accounts.nix
    ./network.nix
    ./desktop.nix
    ./services.nix
    ./pm.nix
    ./installer.nix
    ./testing.nix
  ];

  nixpkgs.overlays = [ (import ../pkgs) ];

  # The image does not include Nix or nixos-rebuild, so the manual is not useful
  # on the installed system.
  documentation.nixos.enable = false;

  # The NixOS release the option defaults above were written against.
  system.stateVersion = "26.05";
}
