# What the pm tree's kernel layer did that nixpkgs leaves to the
# configuration. nixpkgs' common-config.nix already sets what systemd needs
# from the old kernel fragment, cgroups, BPF with BTF, seccomp, PSI and EFI
# among it, and builds the drivers that fragment listed as modules.
#
# Also where the image decides, on each machine, which of the drivers it
# carries that machine gets: nixos-facter reports the hardware early in every
# boot, and losos-hardware (src/losos-hardware) matches the report against
# `losos.hardware.rules` (docs/drivers.md).
{
  config,
  lib,
  pkgs,
  ...
}:

let
  inherit (lib) mkOption types;

  rules = pkgs.writeText "losos-hardware-rules.json" (
    builtins.toJSON {
      rules = lib.mapAttrsToList (name: rule: {
        inherit name;
        inherit (rule)
          class
          vendor
          devices
          load
          blacklist
          fallback
          flags
          ;
        devices_file = if rule.devicesFile == null then null else toString rule.devicesFile;
      }) config.losos.hardware.rules;
    }
  );

  dir = "/run/losos/hardware";
in
{
  options.losos.hardware.rules = mkOption {
    default = { };
    description = ''
      Choices a generic image can only make on the machine: for devices facter
      lists under a class, from a vendor, and optionally only some device IDs,
      which modules to load, which to keep off the device, what to load if the
      chosen driver does not come up, and flags for units to test with
      ConditionPathExists=/run/losos/hardware/flags/<flag>. Most hardware
      needs no rule, since udev loads the one driver the kernel names for it.
    '';
    type = types.attrsOf (
      types.submodule {
        options = {
          class = mkOption {
            type = types.str;
            example = "graphics_card";
            description = "The class nixos-facter lists the device under in its report.";
          };
          vendor = mkOption {
            type = types.ints.u16;
            description = "The PCI or USB vendor ID.";
          };
          devices = mkOption {
            type = types.nullOr (types.listOf types.ints.u16);
            default = null;
            description = "Device IDs that match; with devicesFile also null, any device of the vendor.";
          };
          devicesFile = mkOption {
            type = types.nullOr types.path;
            default = null;
            description = "A JSON array of device IDs, numbers or hex strings, read at boot, for lists that are build products.";
          };
          load = mkOption {
            type = types.listOf types.str;
            default = [ ];
            description = "Modules to load, by name.";
          };
          blacklist = mkOption {
            type = types.listOf types.str;
            default = [ ];
            description = "Modules udev must not load for the device.";
          };
          fallback = mkOption {
            type = types.listOf types.str;
            default = [ ];
            description = "Modules to load when one of load did not.";
          };
          flags = mkOption {
            type = types.listOf types.str;
            default = [ ];
            description = "Flags written to /run/losos/hardware/flags.";
          };
        };
      }
    );
  };

  config = {
    # zswap compresses pages in RAM before they reach losos-swap's encrypted
    # partition, which keeps a machine with little memory usable under load.
    # nixpkgs builds it into the kernel and leaves it off. The NixOS module sets
    # it on the command line and in sysfs, with zstd as the compressor.
    boot.zswap.enable = true;

    # GPU firmware from linux-firmware. amdgpu needs it to start any AMD GPU,
    # and nouveau needs NVIDIA's GSP firmware for Turing and newer. Without it
    # both fall back to llvmpipe. These are prebuilt vendor blobs, because no
    # source for them exists (docs/nixos.md, "What this trusts from outside").
    hardware.enableRedistributableFirmware = true;

    # Disks behind virtio, which is how QEMU, `nix run .#vm` and most clouds
    # attach them. NixOS's default initrd list covers SATA, NVMe and USB but no
    # virtio, so the image booted on a virtio disk waited out every partition
    # it looked for and dropped to emergency mode before repart ever ran.
    boot.initrd.availableKernelModules = [
      "virtio_pci"
      "virtio_blk"
      "virtio_scsi"
    ];

    # NVIDIA cards older than Turing keep nouveau, with Mesa's NVK for Vulkan;
    # newer ones get NVIDIA's open module when nvidia.nix's rule matches.

    # Probe the hardware and write this boot's driver choices, before udev's
    # coldplug could load a driver a rule displaces and before
    # systemd-modules-load reads the ones it wants. That is every boot rather
    # than once at install: a card swapped in gets its driver on the boot it
    # first appears, and the probe is quick, since it reads PCI, USB and CPU
    # facts from sysfs and skips facter's slow items (monitors over DDC, disks)
    # and the udev database, which is not up yet. It needs nothing mounted
    # beyond what the initrd mounted.
    # The rules as the boot units read them, for `losos-hardware plan` by hand.
    environment.etc."losos/hardware-rules.json".source = rules;

    systemd.services.losos-hardware = {
      description = "Choose drivers for this machine's hardware";
      wantedBy = [ "sysinit.target" ];
      before = [
        "sysinit.target"
        "systemd-modules-load.service"
        "systemd-udev-trigger.service"
        "shutdown.target"
      ];
      conflicts = [ "shutdown.target" ];
      unitConfig.DefaultDependencies = false;
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        RuntimeDirectory = "losos/hardware";
        RuntimeDirectoryPreserve = true;
        ExecStart = [
          "${lib.getExe pkgs.nixos-facter} --hardware pci,usb,cpu --output ${dir}/facter.json"
          "${lib.getExe pkgs.losos-hardware} plan --rules ${rules} --report ${dir}/facter.json"
        ];
      };
    };

    # When a rule's driver did not load, load its fallback, so a card NVIDIA's
    # module refuses still gets nouveau instead of no driver at all.
    # systemd-modules-load has returned by then, and a module it loaded has
    # bound its device.
    systemd.services.losos-hardware-fallback = {
      description = "Load fallback drivers for hardware whose chosen driver did not load";
      wantedBy = [ "sysinit.target" ];
      after = [
        "losos-hardware.service"
        "systemd-modules-load.service"
      ];
      before = [
        "sysinit.target"
        "shutdown.target"
      ];
      conflicts = [ "shutdown.target" ];
      unitConfig.DefaultDependencies = false;
      path = [ pkgs.kmod ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = "${lib.getExe pkgs.losos-hardware} fallback";
      };
    };
  };
}
