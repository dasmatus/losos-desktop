# The installer medium: an ISO 9660 filesystem holding the live system's Nix
# store as one squashfs, and an appended FAT partition holding its UKI as the
# removable-media boot path.
#
# EFI only, as the OS is. Written to a stick, the ISO reads as a GPT disk with
# that FAT partition as its ESP; burnt to a disc, El Torito points the
# firmware at the same partition. The firmware starts the UKI directly, with
# no boot loader in front: there is one entry and nothing to choose. nixpkgs'
# iso-image.nix is not used because it is built around GRUB and syslinux, and
# around a Nix that registers the store at boot, none of which is here.
{
  config,
  lib,
  pkgs,
  modulesPath,
  ...
}:

let
  inherit (pkgs.stdenv.hostPlatform) efiArch;
  inherit (config.system.boot.loader) ukiFile;
  inherit (config.system.image) version;

  # The label the initrd finds the medium by. ISO 9660 allows 32 characters.
  volumeID = "losos-${version}-${pkgs.stdenv.hostPlatform.parsed.cpu.name}";

  squashfs = pkgs.callPackage "${modulesPath}/../lib/make-squashfs.nix" {
    fileName = "nix-store";
    storeContents = [ config.system.build.toplevel ];
    # zstd decompresses several times faster than xz for a few percent more
    # size, and everything the installer runs is read from here.
    comp = "zstd -Xcompression-level 19";
  };

  # Sized from its contents as nixpkgs sizes its own EFI image: a tenth more
  # for FAT's own overhead, rounded up to a whole MiB.
  esp =
    pkgs.runCommand "losos-installer-esp.img"
      {
        nativeBuildInputs = with pkgs.buildPackages; [
          dosfstools
          mtools
        ];
      }
      ''
        size=$(du --block-size=1 --apparent-size ${config.system.build.uki}/${ukiFile} | cut -f1)
        size=$(( (size * 110 / 100 / 1048576 + 1) * 1048576 ))
        truncate --size=$size $out
        mkfs.vfat --invariant -i 4c4f534f -n LOSOS-EFI $out
        mmd -i $out ::/EFI ::/EFI/BOOT
        mcopy -i $out ${config.system.build.uki}/${ukiFile} ::/EFI/BOOT/BOOT${lib.toUpper efiArch}.EFI
        fsck.vfat -n $out
      '';
in
{
  assertions = [
    {
      assertion = lib.stringLength volumeID <= 32;
      message = "The installer's volume ID ${volumeID} is longer than ISO 9660's 32 characters.";
    }
  ];

  # The root is a tmpfs; the store is the squashfs with a tmpfs over it, as on
  # nixpkgs' own ISOs. The writable layer is what lets a unit's generated files
  # and the like land in /nix/store at runtime without touching the medium.
  fileSystems = {
    "/" = {
      fsType = "tmpfs";
      options = [ "mode=0755" ];
    };
    "/iso" = {
      device = "/dev/disk/by-label/${volumeID}";
      fsType = "iso9660";
      neededForBoot = true;
      noCheck = true;
    };
    "/nix/.ro-store" = {
      fsType = "squashfs";
      device = "/sysroot/iso/nix-store.squashfs";
      options = [
        "loop"
        "threads=multi"
      ];
      neededForBoot = true;
    };
    "/nix/.rw-store" = {
      fsType = "tmpfs";
      options = [ "mode=0755" ];
      neededForBoot = true;
    };
    "/nix/store" = {
      overlay = {
        lowerdir = [ "/nix/.ro-store" ];
        upperdir = "/nix/.rw-store/store";
        workdir = "/nix/.rw-store/work";
      };
    };
  };

  boot.initrd.availableKernelModules = [
    "squashfs"
    "iso9660"
    "sr_mod"
    "usb_storage"
    "uas"
    "overlay"
  ];
  boot.initrd.kernelModules = [
    "loop"
    "overlay"
  ];

  system.build.isoImage =
    pkgs.runCommand "losos-installer_${version}_${pkgs.stdenv.hostPlatform.parsed.cpu.name}.iso"
      {
        nativeBuildInputs = [ pkgs.buildPackages.xorriso ];
      }
      ''
        xorriso -as mkisofs \
          -iso-level 3 \
          -full-iso9660-filenames \
          -joliet -joliet-long \
          -rational-rock \
          -volid ${volumeID} \
          -appid "LosOS Desktop installer" \
          -partition_offset 16 \
          -append_partition 2 C12A7328-F81F-11D2-BA4B-00A0C93EC93B ${esp} \
          -appended_part_as_gpt \
          -e --interval:appended_partition_2:all:: \
          -no-emul-boot \
          -graft-points \
          -output $out \
          nix-store.squashfs=${squashfs}
      '';
}
