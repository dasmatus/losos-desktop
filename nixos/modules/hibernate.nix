# Hibernation, on laptops.
#
# The swap partition disk.nix describes is encrypted with a fresh random key
# every boot, so nothing written to it outlives a power cut. That is also why
# it cannot hold a hibernation image: the key a resume would need is gone
# with the RAM it lived in. On a laptop booted through a UKI that systemd-stub
# measured into a TPM, this module seals the key to that TPM instead, and
# the rest is systemd's own:
#
#   boot     losos-hibernate-swap formats the swap partition as LUKS2 with a
#            new random key, seals it to the TPM's PCRs 4, 7 and 12 with
#            systemd-cryptenroll and forgets it. The partition's old contents
#            become unreadable, exactly as with the per-boot key.
#   sleep    systemd-sleep writes the image to /dev/mapper/swap and records
#            where it is in the HibernateLocation EFI variable, with
#            autoSwap set because the partition is the disk's swap partition.
#            The hook below asks GRUB to boot the same UKI next time.
#   resume   systemd-hibernate-resume-generator in the initrd reads the
#            variable, unseals the key from the TPM to open
#            /dev/disk/by-designator/swap-luks, and the kernel reads the image.
#
# Everything else -- a desktop, a BIOS PC, a laptop without a TPM -- keeps the
# per-boot key, and logind reports that it cannot hibernate.
{
  config,
  lib,
  pkgs,
  utils,
  ...
}:

let
  systemd = config.systemd.package;

  partition = "/dev/disk/by-partlabel/losos-swap";

  losos-hibernate = pkgs.writeShellApplication {
    name = "losos-hibernate";
    runtimeInputs = [
      pkgs.coreutils
      pkgs.gnused
      pkgs.cryptsetup
      pkgs.util-linux
      systemd
      config.system.build.grub.grubenv
    ];
    text = ''
      # Whether this machine hibernates: a laptop, tablet or convertible,
      # booted through a UKI that systemd-stub measured into a TPM.
      able() {
        # The variable ConditionSecurity=measured-uki reads. systemd-stub
        # sets it only once it has measured the UKI into a TPM, so it also
        # says there is a TPM to seal to, and that this is a UEFI boot,
        # without which systemd-sleep has nowhere to record the image.
        [ -e /sys/firmware/efi/efivars/StubPcrKernelImage-4a67b082-0a4c-41cf-b6c7-440b29bb8c4f ] || return 1
        case "$(chassis)" in
          laptop | convertible | tablet) return 0 ;;
          *) return 1 ;;
        esac
      }

      # The chassis as systemd-hostnamed works it out, in its order: what
      # /etc/machine-info says, then SMBIOS, then ACPI, then the devicetree.
      # hostnamed itself is on the bus, which is not up this early.
      chassis() {
        local value=""
        if [ -r /etc/machine-info ]; then
          value=$(sed -n 's/^CHASSIS=["'"'"']\{0,1\}\([a-z]*\).*/\1/p' /etc/machine-info | tail -n1)
        fi
        if [ -n "$value" ]; then
          echo "$value"
          return
        fi
        if [ -r /sys/class/dmi/id/chassis_type ]; then
          # SMBIOS 3.x, 7.4.1.
          case "$(cat /sys/class/dmi/id/chassis_type)" in
            8 | 9 | 10 | 14) echo laptop; return ;;
            30 | 32) echo tablet; return ;;
            31) echo convertible; return ;;
          esac
        fi
        if [ -r /sys/firmware/acpi/pm_profile ]; then
          # The FADT's Preferred_PM_Profile: 2 is Mobile, 8 is Tablet.
          case "$(cat /sys/firmware/acpi/pm_profile)" in
            2) echo laptop; return ;;
            8) echo tablet; return ;;
          esac
        fi
        if [ -r /proc/device-tree/chassis-type ]; then
          tr -d '\0' </proc/device-tree/chassis-type
        fi
      }

      # A LUKS2 swap whose only key is sealed to this boot's PCRs 4, 7 and 12.
      # It runs as the test of an if, where the shell does not stop at a failed
      # command, so every step says what a failure does.
      sealed() {
        local key
        # The initrd opens the partition when it looks for an image to
        # resume from. A boot that got this far did not resume, so whatever
        # it opened is stale, and this is about to replace it.
        if [ -e /dev/mapper/swap ]; then
          cryptsetup close swap || return 1
        fi
        # 256 random bits, as hex so the shell can hold them. With that much
        # entropy a slow key derivation buys nothing, and PBKDF2's minimum
        # keeps the initrd's unlock instant.
        key=$(od -An -tx1 -N32 -v /dev/urandom | tr -d ' \n') || return 1
        printf %s "$key" | cryptsetup luksFormat --batch-mode --type luks2 \
          --pbkdf pbkdf2 --pbkdf-force-iterations 1000 --key-file - ${partition} || return 1
        printf %s "$key" | cryptsetup open --key-file - ${partition} swap || return 1
        # PCR 4 holds the boot loader and the UKI, so the key unseals only
        # for the UKI that sealed it. PCR 12 holds what systemd-stub was
        # handed besides: without Secure Boot it takes a command line from
        # the boot menu, and one that started a shell must get no key. PCR 7
        # holds the Secure Boot state. The key slot it was formatted with
        # goes, so the TPM is the only way in. systemd-cryptenroll reads the
        # key to unlock with from $PASSWORD.
        PASSWORD=$key systemd-cryptenroll --tpm2-device=auto --tpm2-pcrs=4+7+12 \
          --wipe-slot=password ${partition} || return 1
        make_swap
      }

      # The per-boot key crypttab gives every other machine.
      per_boot() {
        if [ -e /dev/mapper/swap ]; then
          cryptsetup close swap
        fi
        cryptsetup open --type plain --cipher aes-xts-plain64 --key-size 512 \
          --key-file /dev/urandom ${partition} swap
        make_swap
      }

      make_swap() {
        mkswap /dev/mapper/swap || return 1
        # As in disk.nix: udev probed the mapping before it had a swap
        # signature, and nothing tells it to look again.
        udevadm trigger --settle --action=change /dev/mapper/swap
      }

      case "''${1-}" in
        able) able ;;
        unable) ! able ;;
        swap)
          # A TPM that refuses (one in lockout, say) must not cost the
          # machine its swap: fall back to the key crypttab would have used.
          if ! sealed; then
            echo "losos-hibernate: could not seal the swap key to the TPM, using a per-boot key" >&2
            per_boot
          fi
          ;;
        sleep)
          # systemd-sleep runs this as a hook: "pre" or "post", then the
          # operation. A resume must start the kernel that hibernated: Linux
          # refuses an image from another one, and the key is sealed to its
          # UKI. sysupdate may have installed a newer UKI since this boot,
          # and GRUB boots the newest, so ask for this version, once
          # (grub.cfg's losos_oneshot). A disk installed before GRUB still
          # starts systemd-boot, which takes the same request as an EFI
          # variable.
          case "''${2-}" in
            hibernate | hybrid-sleep | suspend-then-hibernate) ;;
            *) exit 0 ;;
          esac
          if losos-grubenv systemd-boot; then
            if [ "''${3-}" = pre ]; then
              bootctl set-oneshot @current
            else
              bootctl set-oneshot ""
            fi
          elif [ "''${3-}" = pre ]; then
            # shellcheck source=/dev/null
            version=$(. /etc/os-release && echo "$IMAGE_VERSION")
            losos-grubenv set losos_oneshot "$version"
          else
            losos-grubenv unset losos_oneshot
          fi
          ;;
        *)
          echo "usage: losos-hibernate able|unable|swap|sleep OPERATION pre|post" >&2
          exit 2
          ;;
      esac
    '';
  };
in
{
  # The initrd must be able to unseal the key and open LUKS: the resume
  # generator calls systemd-cryptsetup with tpm2-device=auto. Nothing is in
  # the initrd's crypttab, so on a boot that is not a resume this adds the
  # tools and opens nothing.
  boot.initrd.luks.forceLuksSupportInInitrd = true;

  # The generator writes this unit only when HibernateLocation names the
  # disk's swap partition. Two changes to it. headless: the TPM is the only
  # key, so a refusal (the firmware changed PCR 7 while the machine slept)
  # must not stop the boot at a passphrase prompt nobody can answer. The
  # leading '-': a failed unlock means no image, not a failed boot.
  boot.initrd.systemd.services."systemd-cryptsetup@swap" = {
    overrideStrategy = "asDropin";
    serviceConfig.ExecStart = [
      ""
      "-${systemd}/bin/systemd-cryptsetup attach swap /dev/disk/by-designator/swap-luks - tpm2-device=auto,headless=true"
    ];
  };

  # Without resume=, the generator waits up to two minutes for the image's
  # device, which a key the TPM will not unseal never makes. This used to
  # cut that to 30 seconds, and the emulated laptop in tests/hibernate.nix
  # had not even started udev by then: a slow machine would have lost every
  # session it hibernated, to save time on the rare boot after a firmware
  # update. systemd's two minutes stay.

  systemd.services.losos-hibernate-swap = {
    description = "Seal the swap partition's key to the TPM for hibernation";
    documentation = [ "https://losos-project.github.io/losos-desktop/hibernation" ];
    # Swap is activated before sysinit.target, so this cannot wait for it.
    unitConfig.DefaultDependencies = false;
    bindsTo = [ "${utils.escapeSystemdPath partition}.device" ];
    after = [
      "${utils.escapeSystemdPath partition}.device"
      "tpm2.target"
    ];
    before = [
      "dev-mapper-swap.swap"
      "swap.target"
      "shutdown.target"
    ];
    conflicts = [ "shutdown.target" ];
    wantedBy = [ "swap.target" ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      ExecCondition = "${lib.getExe losos-hibernate} able";
      ExecStart = "${lib.getExe losos-hibernate} swap";
      # fstab's swap waits 90 seconds for /dev/mapper/swap and then gives
      # up for the rest of the boot. That clock starts at the beginning of
      # the boot, not when this service does, and this service waits for
      # the TPM's device and then for the TPM itself, which on a slow
      # machine (or an emulated one) took longer than that. Asking for the
      # swap again once the mapping exists makes the order not matter;
      # where the first request is still waiting, the two merge.
      ExecStartPost = "${systemd}/bin/systemctl --no-block start dev-mapper-swap.swap";
    };
  };

  # crypttab's per-boot key, for every machine the service above passes over.
  # The mapping is named swap either way, so the fstab line in disk.nix and
  # zswap find it whichever made it.
  systemd.packages = [
    (pkgs.writeTextDir "lib/systemd/system/systemd-cryptsetup@swap.service.d/hibernate.conf" ''
      [Service]
      ExecCondition=${lib.getExe losos-hibernate} unable
    '')
  ];

  environment.etc."systemd/system-sleep/losos-hibernate".source =
    pkgs.writeShellScript "losos-hibernate-sleep" ''
      exec ${lib.getExe losos-hibernate} sleep "$2" "$1"
    '';

  # Closing the lid suspends, and after three hours, or sooner if the
  # battery runs low, the laptop wakes and hibernates. On a machine that
  # cannot hibernate logind suspends instead. On mains power the lid only
  # suspends, and a suspend that began on battery stops counting towards
  # hibernation while plugged in.
  services.logind.settings.Login = {
    HandleLidSwitch = "suspend-then-hibernate";
    HandleLidSwitchExternalPower = "suspend";
  };
  systemd.sleep.settings.Sleep = {
    HibernateDelaySec = "3h";
    HibernateOnACPower = false;
  };

  # A battery that reaches critical while the laptop is awake hibernates it.
  # UPower falls back to powering off where hibernation is not possible.
  services.upower.criticalPowerAction = "Hibernate";
}
