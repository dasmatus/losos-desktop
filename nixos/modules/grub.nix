# GRUB, the boot loader on every PC (docs/boot-loaders.md):
#
#   UEFI  firmware -> ESP: GRUB, menu and theme inside -> chainloads the
#         newest UKI -> systemd-stub -> kernel
#   BIOS  firmware -> MBR -> GRUB's core in the BIOS boot partition
#         (bios.nix) -> the newest UKI's kernel, initrd and command line
#
# Both are the same menu (grub.cfg) and the same theme (nixos/branding), and
# count a new UKI's tries the same way, in grubenv on the ESP, which
# losos-grub-bless settles in the UKIs' file names once a boot succeeds.
#
# UEFI used to boot with systemd-boot, which draws a text menu in the
# firmware's console font and nothing else. GRUB draws the salmon behind
# the menu as the rest of the boot does, from GRUB to Plymouth (splash.nix)
# to the login screen. It is not coming back for that reason, and because
# two loaders that count tries in two places were one more thing to keep in
# step. What systemd-boot gave the rest of the system, GRUB gives too: the
# bli module sets LoaderDevicePartUUID, so systemd-gpt-auto-generator finds
# the root disk and the ESP as before, and chainloading leaves systemd-stub
# to measure the UKI into the TPM as before (hibernate.nix seals to that).
#
# NixOS's own GRUB module stays off (boot.nix): it installs GRUB into a
# running system and regenerates its menu on every rebuild, and this OS is
# an image that is never rebuilt in place. Nothing on a running system
# rewrites this GRUB either; the image and the installer put it on the ESP.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.losos;
  inherit (cfg) arch;
  inherit (config.image.repart.verityStore) partitionIds;
  id = config.system.image.id;

  art = import ../branding {
    inherit pkgs;
    name = config.system.nixos.distroName;
  };

  menu = pkgs.replaceVars ./grub.cfg { inherit id; };

  # The menu's EFI half: the disk to look for UKIs on is the one GRUB was
  # started from, "hd0" of "hd0,gpt1". bli is the Boot Loader Interface,
  # the EFI variables systemd-boot used to set: LoaderDevicePartUUID, which
  # systemd-gpt-auto-generator finds root and the ESP by, and LoaderInfo,
  # which says GRUB started this boot. It sets them when it loads, as every
  # module built into the image does when GRUB starts.
  efi = import ./grub-image.nix {
    inherit pkgs menu art;
    format =
      {
        x86_64 = "x86_64-efi";
        aarch64 = "arm64-efi";
      }
      .${arch.name};
    output = "grub.efi";
    early = ''
      regexp --set=1:losos_disk '^([^,]+)' "$root"
      normal (memdisk)/grub.cfg
    '';
    modules = [
      "part_gpt"
      "part_msdos"
      "fat"
      "chain"
      "bli"
      "efifwsetup"
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
    ]
    # The framebuffer of firmware that predates GOP, which only x86 has.
    ++ lib.optional (arch.name == "x86_64") "efi_uga";
  };

  # grubenv as grub-editenv creates it: a signature line padded with # to
  # 1024 bytes. GRUB writes it in place and never creates it, so the ESP has
  # to start with one.
  env = pkgs.runCommand "losos-grubenv" { } ''
    {
      printf '# GRUB Environment Block\n'
      head -c 1024 /dev/zero | tr '\0' '#'
    } | head -c 1024 > $out
  '';

  # grubenv from Linux, as GRUB's save_env writes it: the same 1024 bytes
  # rewritten where they are, since GRUB can only write into blocks the file
  # already has. grub-editenv would do the same, and bring all of GRUB into
  # the image for it.
  grubenv = pkgs.writeShellApplication {
    name = "losos-grubenv";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.gnugrep
      config.systemd.package
    ];
    text = ''
      # losos-grubenv set NAME VALUE | unset NAME | reset | systemd-boot
      #
      # set, unset and reset edit the ESP's grubenv; reset empties it.
      # systemd-boot succeeds when systemd-boot started this boot rather
      # than GRUB: a disk installed before GRUB replaced it, whose ESP still
      # starts systemd-boot, and whose boots systemd-bless-boot settles.
      info=/sys/firmware/efi/efivars/LoaderInfo-4a67b082-0a4c-41cf-b6c7-440b29bb8c4f
      if [ "''${1-}" = systemd-boot ]; then
        [ -r "$info" ] || exit 1
        tail -c +5 "$info" | tr -d '\0' | grep -q '^systemd-boot'
        exit
      fi

      file="$(bootctl --print-esp-path)/EFI/losos/grubenv"
      [ -f "$file" ] || { echo "losos-grubenv: no $file" >&2; exit 1; }
      new=$(mktemp)
      trap 'rm -f "$new"' EXIT
      case "''${1-}" in
        set | unset)
          name=''${2:?a variable name}
          {
            printf '# GRUB Environment Block\n'
            grep -v -e '^#' -e "^$name=" "$file" || true
            if [ "$1" = set ]; then
              printf '%s=%s\n' "$name" "''${3?a value}"
            fi
          } > "$new"
          ;;
        reset) printf '# GRUB Environment Block\n' > "$new" ;;
        *)
          echo "usage: losos-grubenv set NAME VALUE | unset NAME | reset | systemd-boot" >&2
          exit 2
          ;;
      esac
      size=$(stat -c %s "$new")
      if [ "$size" -gt 1024 ]; then
        echo "losos-grubenv: $file would outgrow its 1024 bytes" >&2
        exit 1
      fi
      head -c $((1024 - size)) /dev/zero | tr '\0' '#' >> "$new"
      dd if="$new" of="$file" bs=1024 count=1 conv=notrunc status=none
      sync --file-system "$file"
    '';
  };

  # systemd-bless-boot's job for a UKI GRUB started: grub.cfg leaves the
  # trial in grubenv, and this settles it in the file name, where sysupdate
  # reads it.
  bless = pkgs.writeShellApplication {
    name = "losos-grub-bless";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.gnused
      config.systemd.package
      grubenv
    ];
    text = ''
      # systemd-boot counted this boot itself, and systemd-bless-boot
      # settles it; grubenv, if it holds anything, is from a GRUB boot of
      # the same disk on another machine, and is left for that one.
      if losos-grubenv systemd-boot; then
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
          # in the menu however grubenv changes.
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
      # empty.
      losos-grubenv reset
    '';
  };
in
{
  system.build.grub = {
    inherit
      art
      menu
      efi
      env
      grubenv
      ;
  };

  environment.systemPackages = [ grubenv ];

  # The removable-media path, which firmware boots with no boot entry
  # registered, so this image needs no efibootmgr and no install step; and
  # a copy at the path of its own that the Windows installer registers with
  # the firmware, where Windows' fallback owns the first one.
  image.repart.partitions.${partitionIds.esp}.contents = {
    "/EFI/BOOT/BOOT${lib.toUpper arch.efi}.EFI".source = "${efi}/grub.efi";
    "/EFI/losos/grub${arch.efi}.efi".source = "${efi}/grub.efi";
    "/EFI/losos/grubenv".source = env;
  };

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
    unitConfig.DefaultDependencies = false;
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      ExecStart = lib.getExe bless;
    };
  };
}
