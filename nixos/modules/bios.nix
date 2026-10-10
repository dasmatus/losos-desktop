# Legacy BIOS PCs, started by GRUB from the MBR (docs/boot-loaders.md).
#
# UEFI firmware starts GRUB from the ESP (grub.nix). This is for x86_64 PCs
# whose firmware has no UEFI, or has it turned off: the same disk, the same
# /usr slots, the same menu and theme and the same updates, started by GRUB
# from the MBR instead. Every disk the image or the installer lays out
# carries both, so one disk starts on either kind of firmware.
#
#   BIOS -> MBR (GRUB's boot.img) -> core.img in the BIOS boot partition,
#     menu and theme built in -> kernel + initrd + command line of the
#     newest UKI's version -> the same initrd and system as under UEFI
#
# BIOS GRUB cannot run a UKI, which is an EFI program, so every release
# also carries each UKI's kernel, initrd and command line as files of their
# own, which sysupdate installs beside the UKI (transfers below). GRUB still
# reads versions and tries from the UKIs' names, and counts tries in
# grubenv, as it does on UEFI; losos-grub-bless (grub.nix) writes the
# outcome back into those names, where sysupdate reads it.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.losos;
  inherit (cfg) arch version;
  id = config.system.image.id;
  inherit (config.system.boot.loader) ukiFile;
  inherit (config.system.build) grub;

  # core.img for the BIOS boot partition, with grub.cfg, the theme and
  # what they use built in: the BIOS disk driver, GPT and FAT to find the
  # files, linux to start them, regexp for the device globs and file names,
  # loadenv for the counter, serial for a console a VM can read, memdisk
  # and tar for the menu itself (grub-image.nix adds the theme's). The
  # early config sets losos_disk to $root, which before the menu is the
  # disk the BIOS started GRUB from.
  core = import ./grub-image.nix {
    inherit pkgs;
    inherit (grub) menu art;
    format = "i386-pc";
    output = "core.img";
    early = ''
      set losos_disk="$root"
      normal (memdisk)/grub.cfg
    '';
    modules = [
      "biosdisk"
      "part_gpt"
      "part_msdos"
      "fat"
      "linux"
      "regexp"
      "test"
      "true"
      "loadenv"
      "echo"
      "normal"
      "memdisk"
      "tar"
      "configfile"
      "reboot"
      "halt"
      "sleep"
      "serial"
      "terminal"
    ];
  };

  # boot.img for the MBR, and core.img.
  bootCode = pkgs.runCommand "losos-grub-bios" { } ''
    mkdir $out
    cp ${core}/core.img $out/core.img
    cp ${pkgs.grub2}/lib/grub/i386-pc/boot.img $out/boot.img
  '';

  install = pkgs.writeShellApplication {
    name = "losos-grub-bios-install";
    runtimeInputs = with pkgs; [
      coreutils
      util-linux
      jq
    ];
    text = ''
      boot_code=${bootCode}
    ''
    + builtins.readFile ./grub-bios-install.sh;
  };

  # The UKI's kernel, initrd and command line, for GRUB to start what the
  # UKI would. Cut from the finished UKI rather than taken from the
  # configuration, because only the finished UKI's command line carries the
  # usrhash= of the /usr it belongs to.
  #
  # root=gpt-auto becomes root=PARTLABEL=: systemd finds the root partition
  # by type on the disk the boot loader names in LoaderDevicePartUUID, an
  # EFI variable, and a BIOS has none to set. repart labels the root
  # partition it makes with its type's name (disk.nix gives it no label).
  files =
    pkgs.runCommand "${id}_${version}-bios"
      {
        nativeBuildInputs = with pkgs.buildPackages; [
          jq
          systemdUkify
        ];
      }
      ''
        cmdline=$(ukify --json=short inspect ${config.system.build.uki}/${ukiFile} \
          | jq -er '.".cmdline".text' | tr -d '\0\n')
        case " $cmdline " in
          *" usrhash="*" root=gpt-auto "* | *" root=gpt-auto "*" usrhash="*) ;;
          *) echo "unexpected UKI command line: $cmdline" >&2; exit 1 ;;
        esac
        cmdline=''${cmdline/root=gpt-auto/root=PARTLABEL=${arch.root}}

        mkdir $out
        cp ${config.boot.uki.settings.UKI.Linux} $out/${id}_${version}.linux
        cp ${config.boot.uki.settings.UKI.Initrd} $out/${id}_${version}.initrd
        printf "set losos_cmdline='%s'\n" "$cmdline" > $out/${id}_${version}.grub
      '';

  # Each of those three as a sysupdate transfer, installed beside the UKI in
  # $BOOT/EFI/Linux, where grub.cfg reads only *.efi names and finds these
  # by the UKI's.
  # They belong to the same version set as the UKI and the /usr slot
  # (update.nix), so a version is installed with all of them or not at all.
  transfer = suffix: {
    Transfer = {
      ProtectVersion = "%A";
      Verify = cfg.update.pubring != null;
    };
    Source = {
      Type = "url-file";
      Path = cfg.update.baseUrl;
      MatchPattern = "${id}_@v.${suffix}";
    };
    Target = {
      Type = "regular-file";
      Path = "/EFI/Linux";
      PathRelativeTo = "boot";
      MatchPattern = "${id}_@v.${suffix}";
      Mode = "0444";
      InstancesMax = 2;
    };
  };

  # systemd-gpt-auto-generator mounts the ESP only on an EFI boot: without
  # LoaderDevicePartUUID it cannot tell which ESP the boot came from. A BIOS
  # boot came from the disk root is on, so this mounts that disk's ESP at
  # /efi, where gpt-auto would have, and sysupdate and bootctl find $BOOT
  # there. On a UEFI boot it writes nothing and gpt-auto does the job.
  espGenerator = pkgs.writeShellScript "losos-bios-esp-generator" ''
    [ -d /sys/firmware/efi ] && exit 0
    PATH=${lib.makeBinPath [ pkgs.util-linux ]}
    root=$(findmnt --noheadings --output SOURCE --nofsroot /) || exit 0
    disk=$(lsblk --noheadings --nodeps --output PKNAME "$root") || exit 0
    [ -n "$disk" ] || exit 0
    esp=$(lsblk --noheadings --list --output PARTTYPE,PARTUUID "/dev/$disk" \
      | while read -r type uuid; do
          if [ "$type" = c12a7328-f81f-11d2-ba4b-00a0c93ec93b ]; then
            echo "$uuid"
            break
          fi
        done)
    [ -n "$esp" ] || exit 0
    cat > "$1/efi.mount" <<EOF
    # Written by losos-bios-esp-generator (bios.nix).
    [Unit]
    Description=EFI System Partition
    Documentation=man:systemd-gpt-auto-generator(8)
    Before=local-fs.target

    [Mount]
    What=/dev/disk/by-partuuid/$esp
    Where=/efi
    Type=vfat
    Options=umask=0077,noexec,nosuid,nodev
    EOF
    mkdir -p "$1/local-fs.target.wants"
    ln -sf ../efi.mount "$1/local-fs.target.wants/efi.mount"
  '';
in
lib.mkIf (arch.name == "x86_64") {
  system.build = {
    grubBiosInstall = install;
    biosBootFiles = files;
  };

  # Where core.img goes: 1M, the size grub-install asks for, and far more
  # than the ~300K core.img is with the theme inside. Before slot A's /usr, which grows into the
  # space behind it on first boot (disk.nix). disk.nix has no definition of
  # its own for it, and repart leaves alone a partition no definition
  # matches.
  image.repart.partitions."05-bios-boot".repartConfig = {
    Type = "21686148-6449-6e6f-744e-656564454649";
    Label = "BIOS boot";
    SizeMinBytes = "1M";
    SizeMaxBytes = "1M";
  };

  systemd.sysupdate.transfers = {
    "40-bios-linux" = transfer "linux";
    "41-bios-initrd" = transfer "initrd";
    "42-bios-grub" = transfer "grub";
  };

  systemd.generators.losos-bios-esp = espGenerator;
}
