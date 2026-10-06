# The Android half of the GSI: the device's vendor partitions mapped out of
# `super` and mounted where bionic expects them, their kernel modules loaded,
# Halium's generic system image running the vendor HALs in an LXC container,
# and libhybris so the desktop's EGL reaches the vendor GPU driver.
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
  inherit (config.losos.halium) loadModules;

  # Android's /data. On a phone that is the userdata partition, which here
  # is this OS's root, so Android gets a directory of it.
  androidData = "/var/lib/android/data";

  # The vendor's logical partitions, mapped by losos-gsi-super from this
  # slot of `super` under their names without the slot suffix. Every one is
  # optional: an older device has no vendor_dlkm, many have no odm.
  vendorPartitions = [
    "vendor"
    "odm"
    "vendor_dlkm"
    "system_dlkm"
  ];

  mapSuper = pkgs.writeShellApplication {
    name = "losos-gsi-map-super";
    runtimeInputs = [
      pkgs.android-tools
      pkgs.gawk
      pkgs.gnugrep
      pkgs.coreutils
      (lib.getBin pkgs.lvm2)
    ];
    text = builtins.readFile ./map-super.sh;
  };

  # Android's init expects to be PID 1 of a system whose /dev it shares and
  # whose network is the host's. The system image is mounted read-only, so
  # nothing here asks LXC to create a mount point inside it; the GSI's root
  # already has /dev, /data, /vendor, /odm and both dlkm directories.
  #
  # This container is host root, not isolation: no user namespace and the
  # host's whole /dev, as in Halium's lxc-android, because which nodes a
  # vendor's HALs open differs per device and ueventd creates them at run
  # time. The vendor blobs are trusted as much as the kernel. What can go
  # without a known HAL needing it does: loading modules (losos-gsi-modules
  # does that on the host), raw I/O ports, the clock, and changing MAC policy.
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
    lxc.mount.entry = /odm odm none bind,optional 0 0
    lxc.mount.entry = /vendor_dlkm vendor_dlkm none bind,optional 0 0
    lxc.mount.entry = /system_dlkm system_dlkm none bind,optional 0 0
    lxc.mount.entry = ${androidData} data none bind 0 0
    lxc.apparmor.profile = unconfined
    lxc.cap.drop = sys_module sys_rawio sys_time mac_admin mac_override
    lxc.init.cmd = /init
  '';
in
{
  config = {
    fileSystems =
      # nofail on all of them: a device whose Android side is missing or
      # broken still boots to the desktop, with software rendering, instead
      # of stopping at an emergency shell.
      lib.genAttrs (map (p: "/${p}") vendorPartitions) (where: {
        device = "/dev/mapper${where}";
        fsType = "auto";
        options = [
          "ro"
          "nofail"
          "x-systemd.device-timeout=10s"
        ];
      })
      // {
        # Halium's generic system image, from the store: the same Android
        # on every device, updated with the OS rather than flashed apart.
        "/android/system" = {
          device = "${pkgs.halium-gsi}/system.img";
          fsType = "ext4";
          options = [
            "loop"
            "ro"
            "nofail"
          ];
        };
        # libhybris's linker looks for bionic and its libraries under
        # /system, which in a system-as-root image is a directory one level
        # down.
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

    # The vendor partitions live in `super`, and their mounts above wait for
    # the device-mapper nodes this creates.
    systemd.services.losos-gsi-super = {
      description = "Map the vendor's logical partitions";
      unitConfig.DefaultDependencies = false;
      wantedBy = [ "local-fs-pre.target" ];
      before = [ "local-fs-pre.target" ];
      # Every device this image boots on has super; one that somehow does
      # not waits out the device timeout and boots without its vendor.
      wants = [ "dev-disk-by\\x2dpartlabel-super.device" ];
      after = [ "dev-disk-by\\x2dpartlabel-super.device" ];
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
        ExecStart = "${lib.getExe mapSuper} /dev/disk/by-partlabel/super";
      };
    };

    # The rest of the device's drivers, once their partitions are mounted:
    # the generic kernel's own modules from system_dlkm first, since the
    # vendor's may use them, then the vendor's. The firmware they ask for
    # on probe is on the vendor partition, so the kernel is pointed there
    # first; NixOS's own firmware path is for a kernel it built.
    systemd.services.losos-gsi-modules = {
      description = "Load the vendor's kernel modules";
      wantedBy = [ "multi-user.target" ];
      before = [
        "losos-android.service"
        "display-manager.service"
      ];
      # Wants, not Requires: most devices lack one partition or another.
      unitConfig.WantsMountsFor = map (p: "/${p}") vendorPartitions;
      script = ''
        if [ -w /sys/module/firmware_class/parameters/path ]; then
          echo -n /vendor/firmware > /sys/module/firmware_class/parameters/path
        fi
        exec ${lib.getExe loadModules} /system_dlkm/lib/modules/* /vendor_dlkm/lib/modules /vendor/lib/modules
      '';
      serviceConfig = {
        Type = "oneshot";
        RemainAfterExit = true;
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
      # Android's system server owns /data as uid 1000.
      "${androidData}".d = {
        mode = "0771";
        user = "1000";
        group = "1000";
      };
    };

    systemd.services.losos-android = {
      description = "Android HAL container";
      documentation = [ "man:lxc-start(1)" ];
      wantedBy = [ "multi-user.target" ];
      # The display manager starts after the HALs, so the compositor finds
      # the GPU's gralloc and hwcomposer services already registered.
      before = [ "display-manager.service" ];
      after = [
        "dev-binderfs.mount"
        "losos-gsi-modules.service"
      ];
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

    # Access to the GPU and its buffer allocators for whoever holds the
    # seat. uaccess, not a group: logind grants it to the active session and
    # takes it back on switch. These are the names every Adreno and Mali
    # driver uses; a node a vendor names otherwise is not opened by the
    # desktop yet (docs/halium.md).
    services.udev.extraRules = ''
      SUBSYSTEM=="misc", KERNEL=="kgsl-3d0|mali[0-9]*|ion", TAG+="uaccess"
      SUBSYSTEM=="dma_heap", TAG+="uaccess"
    '';

    environment.systemPackages = [
      # getprop and setprop, to read what Android's init reports.
      pkgs.libhybris
      pkgs.android-tools
    ];
  };
}
