# LosOS Desktop on a Halium phone or tablet.
#
# Halium is the Android hardware layer, cut down to what a GNU/Linux system
# needs from it: the device's own kernel, its vendor partition of bionic-linked
# HAL libraries and firmware, and a minimal Android system image whose init
# starts those HALs. Everything above the kernel that a person sees is still
# this OS: base.nix's desktop, accounts, network and pm, the same as on a PC.
#
# What differs from the PC build is only how it boots and where it lives:
#
#   bootloader -> boot.img (the device's kernel + NixOS's systemd initrd)
#     -> systemd in the initrd mounts userdata and loop-mounts rootfs.img
#     -> switch-root -> systemd, which starts Android's init in an LXC
#        container (android.nix) beside the desktop
#
# Halium's reference rootfs boots through halium-boot, a busybox ramdisk. This
# keeps NixOS's systemd initrd in its place, so every stage after the kernel is
# systemd here too, and the boot image is built from the configuration rather
# than taken from a port.
#
# There is no UEFI, no verity /usr and no sysupdate: an Android bootloader
# loads boot.img from the boot partition and nothing else, and the root
# filesystem is a file on userdata. docs/nixos.md, "Halium", says what a device
# port supplies and what is not done.
{
  config,
  lib,
  pkgs,
  modulesPath,
  ...
}:

let
  cfg = config.losos.halium;
  inherit (config.losos) version;
  inherit (lib) mkOption types;

  toplevel = config.system.build.toplevel;
  kernelPath = "${config.boot.kernelPackages.kernel}/${config.system.boot.loader.kernelFile}";
  initrdPath = "${config.system.build.initialRamdisk}/${config.system.boot.loader.initrdFile}";

  # Where the initrd mounts userdata. /run survives switch-root, so the mount
  # is still there afterwards, and android.nix hands it to Android as /data.
  userdataMount = config.losos.halium.userdataMount;
in
{
  imports = [
    ../modules/base.nix
    ./android.nix
  ];

  options.losos.halium = {
    userdataMount = mkOption {
      type = types.path;
      default = "/run/halium/userdata";
      readOnly = true;
      internal = true;
      description = "Where the initrd mounts userdata, shared with android.nix.";
    };

    kernelPackages = mkOption {
      type = types.nullOr types.raw;
      default = null;
      example = lib.literalExpression ''
        pkgs.linuxPackagesFor (pkgs.linuxManualConfig {
          version = "5.10.198";
          src = ./kernel-src;
          configfile = ./halium_defconfig;
        })
      '';
      description = ''
        The device's kernel, as a package set. A Halium device boots only the
        kernel its port built from the vendor's tree, so this is the one thing
        every device has to give. Left null, the boot image carries nixpkgs'
        generic kernel, which evaluates and builds but boots no Halium device;
        the build warns.
      '';
    };

    dtb = mkOption {
      type = types.nullOr types.path;
      default = null;
      description = ''
        The device tree blob mkbootimg puts in the boot image (header v2), for
        devices whose bootloader does not take it from a dtb partition or from
        a kernel with the DTB appended.
      '';
    };

    mkbootimgArgs = mkOption {
      type = types.listOf types.str;
      # Header v2 is the first to carry a DTB, and mkbootimg refuses v2
      # without one, so the default follows whether a DTB was given.
      default = [
        "--header_version"
        (if cfg.dtb != null then "2" else "0")
        "--pagesize"
        "4096"
      ];
      defaultText = lib.literalExpression ''[ "--header_version" (if dtb != null then "2" else "0") "--pagesize" "4096" ]'';
      example = [
        "--header_version"
        "2"
        "--base"
        "0x00000000"
        "--kernel_offset"
        "0x00008000"
        "--ramdisk_offset"
        "0x01000000"
        "--tags_offset"
        "0x00000100"
        "--dtb_offset"
        "0x01f00000"
        "--pagesize"
        "4096"
      ];
      description = ''
        The rest of mkbootimg's command line: header version, page size and
        load offsets. These are the device's BOARD_MKBOOTIMG_ARGS from its
        Android BoardConfig.mk, and a bootloader that is given the wrong ones
        refuses the image or jumps to the wrong address, so a port copies them
        verbatim.
      '';
    };

    userdata = mkOption {
      type = types.str;
      default = "/dev/disk/by-partlabel/userdata";
      description = "The userdata partition, which holds rootfs.img.";
    };

    userdataFsType = mkOption {
      type = types.str;
      default = "ext4";
      example = "f2fs";
      description = "The filesystem on userdata, as the device's Android formatted it.";
    };

    rootfsImage = mkOption {
      type = types.str;
      default = "losos/rootfs.img";
      description = ''
        rootfs.img's path inside userdata. It is a file on Android's data
        partition rather than a partition of its own, as on every Halium
        port, because the partition table belongs to the vendor and the only
        partition big enough to hold an OS is userdata.
      '';
    };

    rootfsSize = mkOption {
      type = types.str;
      default = "16G";
      description = ''
        What first boot grows rootfs.img to. The image is built only as large
        as the system closure; the initrd allocates the file to this size
        before mounting it, and x-systemd.growfs grows the filesystem to
        match. If userdata has no room for it, the file keeps its size.
      '';
    };
  };

  config = {
    # systemd's README calls 5.10 its minimum baseline and says older kernels
    # are "not supported at all". Most Halium ports of Android 9 and 10 run
    # 4.x vendor kernels, so this is the line that says which devices can
    # carry this OS, at evaluation rather than as a hang on the phone.
    assertions = [
      {
        assertion = lib.versionAtLeast config.boot.kernelPackages.kernel.version "5.10";
        message = "losos.halium: the device kernel is ${config.boot.kernelPackages.kernel.version}, and systemd ${config.systemd.package.version} needs at least 5.10.";
      }
    ];

    warnings = lib.optional (cfg.kernelPackages == null) ''
      losos.halium.kernelPackages is unset, so boot.img carries nixpkgs'
      generic kernel. It evaluates and builds, but no Halium device boots it:
      set the device port's kernel.
    '';

    boot = {
      kernelPackages = lib.mkIf (cfg.kernelPackages != null) cfg.kernelPackages;

      # The Android bootloader is the only bootloader; there is nothing for
      # NixOS to install.
      loader.grub.enable = false;

      initrd = {
        systemd.enable = true;

        # NixOS's default initrd modules are for PCs (SATA, NVMe, USB HID),
        # and a vendor kernel builds few of them. A port that needs a module
        # to reach userdata names it in availableKernelModules.
        includeDefaultModules = false;
        availableKernelModules = [
          "loop"
          "ext4"
        ]
        ++ lib.optional (cfg.userdataFsType == "f2fs") "f2fs";
        # Vendor kernels build what they need into the image; a module the
        # list above names and the kernel has built in, or does not have,
        # is not an error.
        allowMissingModules = true;

        supportedFilesystems.${cfg.userdataFsType} = true;

        systemd.mounts = [
          {
            what = cfg.userdata;
            where = userdataMount;
            type = cfg.userdataFsType;
            options = "rw,noatime";
            requiredBy = [ "sysroot.mount" ];
            before = [ "sysroot.mount" ];
          }
        ];

        # Grow rootfs.img to its full size before it is mounted. Only ever
        # grows: fallocate allocates up to the length and never shrinks a
        # file that is already larger. Allocated, not truncated: a sparse
        # file would let Android's /data and the root filesystem inside it
        # both count the same free blocks, and the loser's writes would fail
        # under the root filesystem. `-` because a userdata partition without
        # the room leaves rootfs.img at its built size, which still boots.
        systemd.extraBin.fallocate = "${pkgs.util-linux}/bin/fallocate";
        systemd.services.losos-halium-grow-rootfs = {
          description = "Grow rootfs.img to its full size";
          unitConfig = {
            RequiresMountsFor = userdataMount;
            DefaultDependencies = false;
          };
          before = [ "sysroot.mount" ];
          requiredBy = [ "sysroot.mount" ];
          serviceConfig = {
            Type = "oneshot";
            ExecStart = "-/bin/fallocate --length ${cfg.rootfsSize} ${userdataMount}/${cfg.rootfsImage}";
          };
        };
      };

      # The kernel loads firmware from the vendor partition itself, so a
      # driver that probes after /vendor is mounted finds its blobs without
      # waiting for Android's ueventd.
      kernelParams = [ "firmware_class.path=/vendor/firmware" ];
    };

    fileSystems."/" = {
      device = "${userdataMount}/${cfg.rootfsImage}";
      fsType = "ext4";
      options = [
        "loop"
        "x-systemd.requires-mounts-for=${userdataMount}"
        "x-systemd.growfs"
      ];
    };

    system.image = {
      id = "losos-desktop";
      inherit version;
    };

    # The root filesystem: the system closure on ext4, nothing else. Stage 2
    # creates /etc, /var and the rest on first boot, as on the PC image's
    # empty root partition.
    system.build.haliumRootfs = pkgs.callPackage "${modulesPath}/../lib/make-ext4-fs.nix" {
      storePaths = [ toplevel ];
      volumeLabel = "losos-root";
    };

    # The kernel and the initrd in the Android boot image format, with the
    # system's init on the command line as the UKI carries it on a PC.
    system.build.haliumBootImage =
      pkgs.runCommand "losos-desktop_${version}_halium-boot.img"
        { nativeBuildInputs = [ pkgs.buildPackages.android-tools ]; }
        ''
          mkbootimg \
            --kernel ${kernelPath} \
            --ramdisk ${initrdPath} \
            ${lib.optionalString (cfg.dtb != null) "--dtb ${cfg.dtb}"} \
            --cmdline ${lib.escapeShellArg "init=${toplevel}/init ${toString config.boot.kernelParams}"} \
            ${lib.escapeShellArgs cfg.mkbootimgArgs} \
            --output $out
        '';

    # What a person flashes: boot.img with fastboot, rootfs.img copied into
    # userdata from recovery (docs/nixos.md, "Halium").
    system.build.haliumImages =
      pkgs.runCommand "losos-desktop_${version}_halium" { nativeBuildInputs = [ pkgs.buildPackages.xz ]; }
        ''
          mkdir -p $out
          cp ${config.system.build.haliumBootImage} $out/boot.img
          xz --threads=$NIX_BUILD_CORES -c ${config.system.build.haliumRootfs} > $out/rootfs.img.xz
          cd $out
          sha256sum -- * > SHA256SUMS
        '';
  };
}
