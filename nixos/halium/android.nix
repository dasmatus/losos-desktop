# The Android half of a Halium device: its system and vendor images mounted
# where bionic expects them, Android's init running the vendor HALs in an LXC
# container, and libhybris so the desktop's EGL reaches the vendor GPU driver.
#
# LXC rather than systemd-nspawn, because Android's init needs the host's own
# /dev: ueventd creates the HAL device nodes, and the host's processes (the
# compositor through libhybris above all) have to open the same nodes. nspawn
# always gives a container a private /dev. Halium's own lxc-android does it
# this way for the same reason. The container is still a systemd service, so
# `systemctl status losos-android` says whether the HALs are up.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.losos.halium.android;
  inherit (lib) mkOption types;

  # Android's init expects to be PID 1 of a system whose /dev it shares and
  # whose network is the host's. The system image is mounted read-only, so
  # nothing here asks LXC to create a mount point inside it; Android's root
  # already has /dev, /data and /vendor.
  #
  # This container is host root, not isolation: no user namespace and the
  # host's whole /dev, as in Halium's lxc-android, because which nodes a
  # vendor's HALs open differs per device and ueventd creates them at run
  # time. The vendor blobs are trusted as much as the kernel. What can go
  # without a known HAL needing it does: loading modules (the host's udev
  # does that), raw I/O ports, the clock, and changing MAC policy.
  lxcConfig = pkgs.writeText "android.conf" ''
    lxc.rootfs.path = /android/system
    lxc.uts.name = android
    lxc.net.0.type = none
    lxc.tty.max = 0
    lxc.pty.max = 1
    lxc.console.path = none
    lxc.autodev = 0
    lxc.mount.auto = cgroup:mixed proc:mixed sys:mixed
    lxc.mount.entry = /dev dev none rbind 0 0
    lxc.mount.entry = /vendor vendor none bind,optional 0 0
    lxc.mount.entry = ${config.losos.halium.userdataMount} data none bind 0 0
    lxc.apparmor.profile = unconfined
    lxc.cap.drop = sys_module sys_rawio sys_time mac_admin mac_override
    lxc.init.cmd = /init
  '';
in
{
  options.losos.halium.android = {
    system = mkOption {
      type = types.str;
      default = "/dev/disk/by-partlabel/system";
      example = "/dev/disk/by-partlabel/system_a";
      description = ''
        The Halium system image: Android's system-as-root, with /init at its
        top. A/B devices name their slot. On a device with dynamic partitions
        it is a logical partition inside `super`, which nothing here maps yet
        (docs/nixos.md, "Halium").
      '';
    };

    vendor = mkOption {
      type = types.str;
      default = "/dev/disk/by-partlabel/vendor";
      description = "The vendor partition: HAL libraries, firmware and their init scripts.";
    };

    udevRules = mkOption {
      type = types.lines;
      default = "";
      description = ''
        The device's udev rules, generated from its ueventd.rc. Android's
        ueventd gives each device node an owner and mode its HALs rely on;
        these give the host the same, for the nodes the desktop opens itself.
      '';
    };
  };

  config = {
    fileSystems = {
      # nofail on all three: a device whose Android side is missing or broken
      # still boots to the desktop, with software rendering, instead of
      # stopping at an emergency shell.
      "/android/system" = {
        device = cfg.system;
        fsType = "auto";
        options = [
          "ro"
          "nofail"
          "x-systemd.device-timeout=10s"
        ];
      };
      "/vendor" = {
        device = cfg.vendor;
        fsType = "auto";
        options = [
          "ro"
          "nofail"
          "x-systemd.device-timeout=10s"
        ];
      };
      # libhybris's linker looks for bionic and its libraries under /system,
      # which in a system-as-root image is a directory one level down.
      "/system" = {
        device = "/android/system/system";
        fsType = "none";
        options = [
          "bind"
          "nofail"
        ];
        depends = [ "/android/system" ];
      };
    };

    # binderfs, so Android 10 and later get their binder devices from the
    # kernel rather than from static nodes. The names are the ones Android's
    # servicemanagers open.
    systemd.mounts = [
      {
        what = "binder";
        where = "/dev/binderfs";
        type = "binder";
        # Wanted, not required: a kernel without binderfs still has the
        # static nodes the links below would otherwise point to.
        wantedBy = [ "local-fs.target" ];
      }
    ];
    systemd.tmpfiles.settings."10-halium" = {
      "/dev/binder".L.argument = "binderfs/binder";
      "/dev/hwbinder".L.argument = "binderfs/hwbinder";
      "/dev/vndbinder".L.argument = "binderfs/vndbinder";
      "/var/lib/lxc/android".d.mode = "0700";
    };

    systemd.services.losos-android = {
      description = "Android HAL container";
      documentation = [ "man:lxc-start(1)" ];
      wantedBy = [ "multi-user.target" ];
      # The display manager starts after the HALs, so the compositor finds
      # the GPU's gralloc and hwcomposer services already registered.
      before = [ "display-manager.service" ];
      after = [ "dev-binderfs.mount" ];
      wants = [ "dev-binderfs.mount" ];
      unitConfig = {
        RequiresMountsFor = [
          "/android/system"
          "/vendor"
        ];
        ConditionPathExists = "/android/system/init";
      };
      serviceConfig = {
        ExecStart = "${pkgs.lxc}/bin/lxc-start --foreground --name=android --lxcpath=/var/lib/lxc --rcfile=${lxcConfig}";
        ExecStop = "${pkgs.lxc}/bin/lxc-stop --name=android --lxcpath=/var/lib/lxc --kill";
        # Android's init stops its own services on SIGPWR before the
        # container goes, as it would on a phone being turned off.
        KillSignal = "SIGPWR";
        TimeoutStopSec = 30;
      };
    };

    # The vendor GPU driver as one more GLVND vendor. libhybris's EGL is tried
    # first and steps aside on a device with no Android driver, which leaves
    # Mesa (desktop.nix) for one whose kernel has a DRM driver of its own.
    hardware.graphics.extraPackages = [ pkgs.libhybris ];

    # The device's own rules, plus access to the GPU and its buffer
    # allocators for whoever holds the seat. uaccess, not a group: logind
    # grants it to the active session and takes it back on switch.
    services.udev.extraRules = ''
      SUBSYSTEM=="misc", KERNEL=="kgsl-3d0|mali[0-9]*|ion", TAG+="uaccess"
      SUBSYSTEM=="dma_heap", TAG+="uaccess"
      ${cfg.udevRules}
    '';

    environment.systemPackages = [
      # getprop and setprop, to read what Android's init reports.
      pkgs.libhybris
      pkgs.android-tools
    ];
  };
}
