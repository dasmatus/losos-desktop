# First boot creates the disk.
#
# The image carries an ESP and one /usr slot (image.nix) and nothing else.
# systemd-repart runs in the initrd on every boot and makes the disk match the
# definitions below: on the first boot that means growing /usr to its slot
# size, creating the second slot sysupdate will write into, the root partition,
# /home and swap. On every later boot it finds nothing to do.
#
# The pm tree kept its root partition for the operating system and marked only
# /home for a factory reset, and docs/factory-reset.md said why that was the
# conservative half: /etc and /var lived on root with /usr, so a reset could
# not touch them without taking the OS too. Here the OS is /usr and root is
# state only, so root is marked as well, and a factory reset now returns the
# machine to exactly what the image contained.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.losos;
  inherit (cfg) arch;
in
{
  boot.initrd.systemd.repart = {
    enable = true;

    # repart's own definitions are generated into the initrd's /etc/repart.d.
    # losos-swap writes one more into /run/repart.d, because the size of a swap
    # partition is the size of the machine's RAM and that is only known here.
    extraArgs = [ "--definitions=/run/repart.d" ];
  };

  # Upstream orders repart After=initrd-usr-fs.target, and the fstab generator
  # orders /sysusr/usr Before= it, but NixOS's initrd does not ship that target.
  # Ordering against a unit that does not exist is no ordering at all, so repart
  # started before /sysusr/usr was mounted, fell back to looking for the disk
  # under /sysroot, found nothing, and first boot waited out gpt-auto-root for
  # a root partition nobody had made. initrd.target already Wants= the target,
  # so shipping it is all the wiring needed.
  boot.initrd.systemd.additionalUpstreamUnits = [ "initrd-usr-fs.target" ];

  # repart formats what it creates by running mkfs.ext4 and mkswap from the
  # initrd's PATH, and neither was there: no fileSystems entry names root or
  # /home, since gpt-auto finds them, so nothing told NixOS the initrd needs
  # ext4. Declaring it brings e2fsprogs and the module; mkswap comes alone,
  # because the rest of util-linux has no business in the initrd.
  boot.initrd.supportedFilesystems.ext4 = true;
  boot.initrd.systemd.extraBin.mkswap = "${pkgs.util-linux}/bin/mkswap";

  boot.initrd.systemd.services.systemd-repart = {
    # NixOS orders repart after sysroot.mount by default, because without a
    # device argument repart discovers the disk from what is mounted. On the
    # first boot of this image there is no root partition to mount yet -- repart
    # is what creates it -- so that ordering would wait forever on a device that
    # only repart can bring into being. Upstream's own ordering is After=
    # initrd-usr-fs.target, Before=initrd-root-fs.target: repart finds the disk
    # through /sysusr/usr, which gpt-auto mounts from the verity-protected /usr
    # partition before root is looked for. Forcing the NixOS addition away
    # restores exactly that, once the target exists (above).
    after = lib.mkForce [ ];

    # This was conditioned on `!losos.install`, for an installer that booted
    # this image with a tmpfs root and must not repartition its own medium.
    # The installer is a separate live system now (installer.nix), so every
    # boot of this image is a boot of an installed disk.

    serviceConfig.ExecStartPre = "${pkgs.losos-swap}/lib/losos/losos-swap /run/repart.d";
  };

  boot.initrd.systemd.storePaths = [ "${pkgs.losos-swap}/lib/losos/losos-swap" ];

  # The initrd half of a factory reset -- factory-reset-now.target and
  # systemd-factory-reset-complete, which clears the request once repart has
  # recreated every FactoryReset=yes partition -- is in NixOS's initrd already.

  systemd.repart.partitions = {
    "10-esp" = {
      Type = "esp";
      Format = "vfat";
      # Several UKIs side by side, so an A/B rollout never has to repartition.
      SizeMinBytes = "512M";
      SizeMaxBytes = "512M";
    };

    # Slot A: matches the /usr pair the image shipped with, and grows the data
    # half to its full slot size. It can only grow because image.nix places it
    # last, with the free space of the disk directly behind it.
    "20-usr-verity-a" = {
      Type = arch.usrVerity;
      SizeMinBytes = "128M";
      SizeMaxBytes = "128M";
    };
    "21-usr-a" = {
      Type = arch.usr;
      SizeMinBytes = cfg.usrSize;
      SizeMaxBytes = cfg.usrSize;
    };

    # Slot B: created empty. `_empty` is the label systemd-sysupdate looks for
    # when it needs a partition to write a new version into.
    "22-usr-verity-b" = {
      Type = arch.usrVerity;
      Label = "_empty";
      SizeMinBytes = "128M";
      SizeMaxBytes = "128M";
    };
    "23-usr-b" = {
      Type = arch.usr;
      Label = "_empty";
      SizeMinBytes = cfg.usrSize;
      SizeMaxBytes = cfg.usrSize;
    };

    # State: /etc's writable layer, /var, the journal, the machine ID.
    # Type= is what systemd-gpt-auto-generator matches to find it.
    "30-root" = {
      Type = arch.root;
      Format = "ext4";
      FactoryReset = true;
      SizeMinBytes = "8G";
      Weight = 250;
    };

    # /home, as its own partition, so systemd-homed's per-user LUKS images
    # survive an update and have a filesystem of their own to live on.
    "40-home" = {
      Type = "home";
      Format = "ext4";
      FactoryReset = true;
      SizeMinBytes = "4G";
      # No maximum: with the larger weight it takes most of the rest of the disk.
      Weight = 1000;
    };
  };

  # The swap partition losos-swap describes is encrypted with a fresh random
  # key every boot rather than exposing raw swap blocks on disk. The label is
  # fixed by losos-swap. nofail, so a boot that finds no swap partition does
  # not wait for one.
  environment.etc.crypttab.text = ''
    swap /dev/disk/by-partlabel/losos-swap /dev/urandom swap,nofail
  '';

  # Naming the mapping here does two things: it activates /dev/mapper/swap, and
  # a swap line in fstab is what stops systemd-gpt-auto-generator activating
  # the raw partition underneath it.
  swapDevices = [
    {
      device = "/dev/mapper/swap";
      options = [ "nofail" ];
    }
  ];

  # systemd-gpt-auto-generator mounts /home and the ESP (at /efi) from the disk
  # the system booted from, by partition type, which is why neither appears in
  # fileSystems. NixOS runs systemd's own generators as they are. This module
  # used to link this one back in by hand, on the belief that NixOS masked it;
  # it never did, and the masking is only the example in systemd.generators'
  # documentation.

  # The factory reset Varlink API at /run/systemd/io.systemd.FactoryReset.
  # Socket-activated, so enabling it costs a socket and no process. Without it
  # the only way to ask is the kernel command line, which a desktop user cannot
  # reach. NixOS already carries factory-reset.target and its request units.
  systemd.additionalUpstreamSystemUnits = [
    "systemd-factory-reset.socket"
    "systemd-factory-reset@.service"
  ];
  systemd.sockets.systemd-factory-reset.wantedBy = [ "sockets.target" ];

  # Let an active, local administrator ask for a factory reset.
  #
  # Starting factory-reset.target goes through
  # org.freedesktop.systemd1.manage-units, which covers every unit; granting it
  # wholesale to provide a reset button would hand the session every other
  # service on the machine too. So this narrows to that one unit, that one
  # verb, and a session that is on a seat and not remote -- and still answers
  # AUTH_ADMIN, not YES. It decides who may be asked, not who may skip asking.
  security.polkit.extraConfig = ''
    polkit.addRule(function (action, subject) {
      if (action.id !== "org.freedesktop.systemd1.manage-units") {
        return polkit.Result.NOT_HANDLED;
      }
      if (action.lookup("unit") !== "factory-reset.target") {
        return polkit.Result.NOT_HANDLED;
      }
      if (action.lookup("verb") !== "start") {
        return polkit.Result.NOT_HANDLED;
      }
      if (subject.active && subject.local && subject.isInGroup("wheel")) {
        return polkit.Result.AUTH_ADMIN;
      }
      return polkit.Result.NOT_HANDLED;
    });
  '';
}
