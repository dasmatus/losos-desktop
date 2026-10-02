# The installer: the same image, booted with one more word on its command line.
#
# The pm tree's installer was systemd-sysinstall, which systemd gained in
# v261. NixOS 26.05 ships systemd 260, so this does by hand the three things
# sysinstall would do -- pick a disk, run systemd-repart against it, put a
# bootloader on it -- and does them with the same tools. When nixpkgs reaches
# v261 this file should shrink to sysinstall's drop-in and these repart
# definitions.
#
# The installer UKI (image.nix) boots with a tmpfs root, `losos.install`, and
# `systemd.unit=losos-install.target`, which is a target with no display
# manager and no homed first-boot wizard in it: only this.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.losos;
  inherit (cfg) arch;
  inherit (config.system.boot.loader) ukiFile;
  systemdBoot = "${config.systemd.package}/lib/systemd/boot/efi/systemd-boot${arch.efi}.efi";

  loaderConf = pkgs.writeText "loader.conf" ''
    timeout 3
    editor no
  '';

  # Where the script mounts the ESP this system booted from, which is where the
  # ordinary UKI is read from.
  bootedEsp = "/run/losos-install/esp";

  format = pkgs.formats.ini { listsAsDuplicateKeys = true; };
  definition = name: settings: format.generate name { Partition = settings; };

  # The target disk's layout, and the whole of the install.
  #
  # CopyBlocks=auto copies the /usr pair this system is running from, block
  # for block, onto the new disk. So installing means duplicating the medium
  # you booted, and there is no third artifact -- no squashfs, no OS image
  # inside the installer image -- that could drift out of step with what
  # actually boots. The ESP is populated from files instead of copied, because
  # the medium's ESP carries the installer UKI, and the installed system must
  # not boot into the installer.
  #
  # Only slot A and the ESP are created here. Everything else -- slot B, root,
  # /home, swap -- is created by the installed system's own first boot, from
  # disk.nix, which is the same code path an image written with dd takes.
  definitions = pkgs.linkFarm "repart.sysinstall.d" [
    {
      name = "10-esp.conf";
      path = definition "10-esp.conf" {
        Type = "esp";
        Format = "vfat";
        SizeMinBytes = "512M";
        SizeMaxBytes = "512M";
        CopyFiles = [
          "${systemdBoot}:/EFI/BOOT/BOOT${lib.toUpper arch.efi}.EFI"
          "${systemdBoot}:/EFI/systemd/systemd-boot${arch.efi}.efi"
          "${loaderConf}:/loader/loader.conf"
          "${bootedEsp}/EFI/Linux/${ukiFile}:/EFI/Linux/${ukiFile}"
        ];
      };
    }
    {
      name = "20-usr-verity.conf";
      path = definition "20-usr-verity.conf" {
        Type = arch.usrVerity;
        CopyBlocks = "auto";
        SizeMinBytes = "128M";
        SizeMaxBytes = "128M";
      };
    }
    {
      name = "21-usr.conf";
      path = definition "21-usr.conf" {
        Type = arch.usr;
        # No Format=: the copied blocks bring their own filesystem, and
        # formatting would write an empty one over them.
        CopyBlocks = "auto";
      };
    }
  ];

  install = pkgs.writeShellApplication {
    name = "losos-install";
    runtimeInputs = [
      config.systemd.package
      pkgs.coreutils
      pkgs.util-linux
    ];
    text = ''
      # The ESP this medium booted from is the partition the boot loader
      # reported in LoaderDevicePartUUID -- the same variable gpt-auto reads.
      # Read directly rather than through a mount unit, because on a tmpfs
      # root there is no root disk for gpt-auto to start from.
      var=/sys/firmware/efi/efivars/LoaderDevicePartUUID-4a67b082-0a4c-41cf-b6c7-440b29bb8c4f
      uuid=$(tail -c +5 "$var" | tr -d '\0' | tr '[:upper:]' '[:lower:]')
      mkdir -p ${bootedEsp}
      mountpoint -q ${bootedEsp} || mount -o ro "/dev/disk/by-partuuid/$uuid" ${bootedEsp}

      lsblk --nodeps --output NAME,SIZE,MODEL,TRAN --exclude 7,11
      echo

      # systemd-ask-password rather than read: the prompt then goes through the
      # same agent any other boot-time question does, and works the same on a
      # serial console as on tty1.
      disk=$(systemd-ask-password --echo=yes --timeout=0 "Install LosOS Desktop onto which disk (e.g. sda, nvme0n1)?")
      disk=/dev/''${disk#/dev/}

      echo
      systemd-repart --definitions=${definitions} --empty=force --dry-run=yes "$disk"
      echo

      answer=$(systemd-ask-password --echo=yes --timeout=0 "Everything on $disk will be erased. Type 'erase' to continue:")
      if [ "$answer" != erase ]; then
        echo "Nothing was written."
        exit 1
      fi

      systemd-repart --definitions=${definitions} --empty=force --dry-run=no "$disk"
      echo "Installed. Remove the installation medium; the machine will restart."
      systemd-ask-password --echo=yes --timeout=0 "Press Enter to restart." > /dev/null || true
    '';
  };
in
{
  systemd.targets.losos-install = {
    description = "LosOS Desktop installer";
    requires = [ "basic.target" ];
    after = [ "basic.target" ];
    wants = [ "losos-install.service" ];
    unitConfig.AllowIsolate = true;
  };

  systemd.services.losos-install = {
    description = "Install LosOS Desktop onto a disk";
    unitConfig.ConditionKernelCommandLine = "losos.install";
    serviceConfig = {
      Type = "oneshot";
      ExecStart = lib.getExe install;
      StandardInput = "tty";
      StandardOutput = "tty";
      StandardError = "tty";
      TTYPath = "/dev/tty1";
      TTYReset = true;
      TTYVHangup = true;
    };

    # Either way, the machine restarts: into the installed system on success,
    # and back into the installer on failure, which is where the person
    # already is.
    unitConfig.SuccessAction = "reboot";
    unitConfig.FailureAction = "reboot";
  };
}
