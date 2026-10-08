# LosOS Desktop as a NixOS module, for UEFI PCs and arm64 machines. Importing
# this is the whole OS; a host configuration adds nothing but its architecture
# and version (see flake.nix).
#
# docs/nixos.md maps each file here to the piece of the pm tree it replaces.
{
  imports = [
    ./base.nix
    ./boot.nix
    ./bios.nix
    ./hardware.nix
    ./allocator.nix
    ./image.nix
    ./disk.nix
    ./update.nix
    ./installer.nix
    ./windows-installer.nix
    ./testing.nix
  ];
}
