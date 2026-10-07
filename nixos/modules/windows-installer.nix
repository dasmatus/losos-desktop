# The installer for a machine that runs Windows: a Windows program that
# shrinks C: and installs LosOS in the space beside it (docs/windows-installer.md).
#
# Like installer.nix, this is the OS's half: the program is built from this
# configuration's own channel URL, release key, image id, /usr slot size, and
# the systemd-boot and loader.conf image.nix puts on an ESP, so it installs
# exactly what an update of this OS would fetch, into a slot of the size
# first boot expects. A release carries it beside the ISO (image.nix).
#
# x86_64 only. Windows on arm64 runs on Snapdragon laptops this OS has no
# kernel configuration for, so an aarch64 build would install a system that
# does not start there.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.losos;
  inherit (config.image.repart.verityStore) partitionIds;
  espContents = config.image.repart.partitions.${partitionIds.esp}.contents;
in
lib.mkIf (cfg.arch.name == "x86_64") {
  system.build.windowsInstaller =
    pkgs.pkgsCross.mingwW64.callPackage ../pkgs/losos-windows-installer.nix
      {
        updateUrl = cfg.update.baseUrl;
        inherit (cfg.update) pubring;
        imageId = config.system.image.id;
        arch = cfg.arch.name;
        inherit (cfg) usrSize;
        systemdBoot = espContents."/EFI/systemd/systemd-boot${cfg.arch.efi}.efi".source;
        loaderConf = espContents."/loader/loader.conf".source;
      };
}
