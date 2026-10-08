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
          classes
          vendor
          devices
          virtualisation
          load
          blacklist
          fallback
          flags
          reserve
          ;
        devices_file = if rule.devicesFile == null then null else toString rule.devicesFile;
        cpu_vendor = rule.cpuVendor;
      }) config.losos.hardware.rules;
    }
  );

  dir = "/run/losos/hardware";
in
{
  options.losos.hardware.rules = mkOption {
    default = { };
    description = ''
      Choices a generic image can only make on the machine. A rule matches on
      any of three selectors, all of which must hold: a device facter lists
      under one of `classes` (from `vendor`, and optionally only some IDs),
      the hypervisor facter detected (`virtualisation`), and the CPU's vendor
      (`cpuVendor`). When it matches, it loads modules, keeps others off the
      device, names what to load if its driver does not come up, and sets
      flags, which units that only make sense on that hardware test with
      ConditionPathExists=/run/losos/hardware/flags/<flag>. Most drivers need
      no rule, since udev loads the one the kernel names for a device.
    '';
    type = types.attrsOf (
      types.submodule {
        options = {
          classes = mkOption {
            type = types.listOf types.str;
            default = [ ];
            example = [ "graphics_card" ];
            description = "The classes nixos-facter may list a matching device under.";
          };
          vendor = mkOption {
            type = types.nullOr types.ints.u16;
            default = null;
            description = "The PCI or USB vendor ID of a matching device.";
          };
          devices = mkOption {
            type = types.nullOr (types.listOf (types.either types.ints.u16 types.str));
            default = null;
            description = ''
              IDs that match: device IDs of `vendor`'s, as numbers or hex
              strings, or "vvvv:dddd" pairs. With devicesFile also null, any
              device in the classes from `vendor` matches.
            '';
          };
          devicesFile = mkOption {
            type = types.nullOr types.path;
            default = null;
            description = "The same as devices, as a JSON array in a file read at boot, for lists that are build products.";
          };
          virtualisation = mkOption {
            type = types.nullOr (types.listOf types.str);
            default = null;
            example = [ "none" ];
            description = "facter's virtualisation values that match; none is bare metal.";
          };
          cpuVendor = mkOption {
            type = types.nullOr types.str;
            default = null;
            example = "GenuineIntel";
            description = "The CPU vendor string that matches.";
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
          reserve = mkOption {
            type = types.listOf types.str;
            default = [ ];
            description = ''
              Modules only this rule may have loaded, blacklisted at boot when
              it does not match. Not a blacklist in boot.blacklistedKernelModules:
              systemd-modules-load honours blacklists even for the modules it
              loads by name, so the rule could never load them.
            '';
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
          # `-`: if the probe fails, the plan is made from no report, which
          # matches nothing and still keeps every rule's reserved modules off.
          "-${lib.getExe pkgs.nixos-facter} --hardware pci,usb,cpu --output ${dir}/facter.json"
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

    # Services that only make sense on some hardware are in every image and
    # start only where facter found that hardware; the image cannot install
    # a package per machine, but it can leave one idle. Each rule sets a flag
    # its unit tests. A device plugged in after boot, such as a USB reader,
    # counts from the next boot, when the probe sees it.

    # fprintd with a reader libfprint drives, by the list NixOS's
    # hardware.facter module keeps of the IDs this nixpkgs' libfprint
    # supports. fprintd is started over D-Bus, by pam_fprintd and derisk's
    # reader prompt; without a reader that start is refused at once, and
    # both go on to the password.
    losos.hardware.rules.fingerprint = {
      # hwinfo files a reader under fingerprint when it knows the device, and
      # under usb or unknown when it does not.
      classes = [
        "fingerprint"
        "usb"
        "unknown"
      ];
      devicesFile = pkgs.runCommand "libfprint-devices.json" {
        nativeBuildInputs = [ pkgs.jq ];
      } "jq keys ${pkgs.path}/nixos/modules/hardware/facter/fingerprint/devices.json > $out";
      flags = [ "fingerprint" ];
    };
    systemd.services.fprintd.unitConfig.ConditionPathExists = "${dir}/flags/fingerprint";

    # thermald, which keeps an Intel CPU under its thermal limits by stepping
    # through gentler controls than the firmware's emergency throttling, so a
    # laptop runs cooler and quieter under load. It manages nothing on AMD or
    # in a VM.
    losos.hardware.rules.intel-thermald = {
      cpuVendor = "GenuineIntel";
      virtualisation = [ "none" ];
      flags = [ "intel-cpu" ];
    };
    services.thermald.enable = pkgs.stdenv.hostPlatform.isx86;
    systemd.services.thermald.unitConfig.ConditionPathExists = "${dir}/flags/intel-cpu";
  };
}
