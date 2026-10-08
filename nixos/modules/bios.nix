# Legacy BIOS PCs, started by GRUB (docs/boot-loaders.md).
#
# UEFI is how this OS boots, with systemd-boot, and nothing here changes
# that. This is for x86_64 PCs whose firmware has no UEFI, or has it turned
# off: the same disk, the same /usr slots and the same updates, started by
# GRUB from the MBR instead of by systemd-boot from the ESP. Every disk the
# image or the installer lays out carries both, so one disk starts on
# either kind of firmware.
#
#   BIOS -> MBR (GRUB's boot.img) -> core.img in the BIOS boot partition,
#     menu built in -> kernel + initrd + command line of the newest UKI's
#     version -> the same initrd and system as under UEFI
#
# BIOS GRUB cannot run a UKI, which is an EFI program, so every release
# also carries each UKI's kernel, initrd and command line as files of their
# own, which sysupdate installs beside the UKI (transfers below). GRUB still
# reads versions and tries from the UKIs' names, and counts tries in
# grubenv; losos-grub-bless writes the outcome back into those names, so
# sysupdate and systemd-boot see the same state GRUB does.
#
# NixOS's own GRUB module stays off (boot.nix): it installs GRUB into a
# running system and regenerates its menu on every rebuild, and this OS is
# an image that is never rebuilt in place.
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

  grubDir = "${pkgs.grub2}/lib/grub/i386-pc";

  # What grub-bios.cfg uses and nothing else: the BIOS disk driver, GPT and
  # FAT to find the files, linux to start them, regexp for the device globs
  # and file names, loadenv for the counter, serial for a console a VM can
  # read, memdisk and tar for the menu itself.
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

  menu = pkgs.replaceVars ./grub-bios.cfg { inherit id; };

  # boot.img for the MBR and core.img for the BIOS boot partition, with
  # every module and the menu inside it, so GRUB reads nothing of its own
  # from any filesystem. The early config runs before the menu, while $root
  # is still the disk the BIOS started from; the prefix has no device so
  # that it is.
  bootCode =
    pkgs.runCommand "losos-grub-bios"
      {
        nativeBuildInputs = [ pkgs.buildPackages.grub2 ];
      }
      ''
        # The menu fails at boot, not here, unless it is checked here.
        grub-script-check ${menu}
        mkdir memdisk $out
        cp ${menu} memdisk/grub.cfg
        tar -C memdisk -cf memdisk.tar grub.cfg
        cat > early.cfg <<'EOF'
        set losos_disk="$root"
        normal (memdisk)/grub.cfg
        EOF
        grub-mkimage -O i386-pc -d ${grubDir} -m memdisk.tar -c early.cfg -p / \
          -o $out/core.img ${toString modules}
        cp ${grubDir}/boot.img $out/boot.img

        # grubenv as grub-editenv creates it: a signature line padded with #
        # to 1024 bytes. GRUB writes it in place and never creates it, so the
        # ESP has to start with one.
        {
          printf '# GRUB Environment Block\n'
          head -c 1024 /dev/zero | tr '\0' '#'
        } | head -c 1024 > $out/grubenv
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
  # $BOOT/EFI/Linux, where systemd-boot reads only *.efi and so ignores them.
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

  # systemd-bless-boot's job for a UKI GRUB started: grub-bios.cfg leaves
  # the trial in grubenv, and this settles it in the file name, where
  # sysupdate and systemd-boot read it.
  bless = pkgs.writeShellApplication {
    name = "losos-grub-bless";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.gnused
      config.systemd.package
    ];
    text = ''
      # Under UEFI, systemd-boot counted this boot and systemd-bless-boot
      # settles it; grubenv, if it holds anything, is from a BIOS boot of
      # the same disk on another machine, and is left for that one.
      if [ -d /sys/firmware/efi ]; then
        exit 0
      fi

      esp=$(bootctl --print-esp-path)
      boot=$(bootctl --print-boot-path)
      env="$esp/EFI/losos/grubenv"
      [ -f "$env" ] || exit 0

      get() { sed -n "s/^$1=//p" "$env" | head -n 1; }
      entry=$(get losos_entry)
      left=$(get losos_left)
      booted=$(get losos_booted)
      [ -n "$entry" ] || exit 0

      file=
      for dir in "$boot" "$esp"; do
        if [ -e "$dir/EFI/Linux/$entry" ]; then
          file="$dir/EFI/Linux/$entry"
          break
        fi
      done

      if [ -n "$file" ]; then
        dir=$(dirname "$file")
        base=''${entry%%+*}
        if [ "$booted" = "$entry" ]; then
          # The UKI on trial is the one that reached boot-complete.target:
          # it loses its counter, as systemd-bless-boot would leave it.
          mv "$file" "$dir/$base.efi"
          echo "Marked $base.efi as good."
        elif [ "$left" = 0 ]; then
          # Its tries ran out and GRUB started an older UKI instead. The name
          # systemd-boot gives such an entry, +0-<tries made>, keeps it last
          # in both loaders' menus however grubenv changes.
          counter=''${entry#*+}
          counter=''${counter%.efi}
          tries=''${counter%%-*}
          made=0
          if [ "$counter" != "$tries" ]; then
            made=''${counter#*-}
          fi
          mv "$file" "$dir/$base+0-$((tries + made)).efi"
          echo "Marked $base as failed after $((tries + made)) tries."
        else
          # An older UKI picked by hand while the newer one is on trial:
          # the trial goes on at the next boot.
          exit 0
        fi
        sync --file-system "$dir"
      fi

      # The trial is settled, or its UKI is gone, so grubenv goes back to
      # empty, written in place as GRUB writes it.
      cat ${bootCode}/grubenv > "$env"
      sync --file-system "$env"
    '';
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

  # The ESP's grubenv, which the image and the installer both copy from
  # here. BIOS boot needs nothing else on the ESP but the files sysupdate
  # writes.
  image.repart.partitions.${config.image.repart.verityStore.partitionIds.esp}.contents = {
    "/EFI/losos/grubenv".source = "${bootCode}/grubenv";
  };

  # Where core.img goes: 1M, the size grub-install asks for, and far more
  # than the ~180K core.img is. Before slot A's /usr, which grows into the
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

  # As systemd-bless-boot.service is wired: after boot-complete.target,
  # which systemd-boot-check-no-failures gates (boot.nix), pulled in early
  # so a boot that never gets there leaves the trial running.
  systemd.services.losos-grub-bless = {
    description = "Mark the UKI GRUB started as good";
    wantedBy = [ "basic.target" ];
    requires = [ "boot-complete.target" ];
    after = [
      "local-fs.target"
      "boot-complete.target"
    ];
    conflicts = [ "shutdown.target" ];
    before = [ "shutdown.target" ];
    unitConfig = {
      DefaultDependencies = false;
      ConditionFirmware = "!uefi";
    };
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      ExecStart = lib.getExe bless;
    };
  };
}
