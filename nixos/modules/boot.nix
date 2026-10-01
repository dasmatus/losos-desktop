# How it boots. Every stage is systemd, as in docs/boot.md:
#
#   firmware -> systemd-boot -> UKI (systemd-stub) -> kernel
#     -> systemd in the initrd: repart, veritysetup, gpt-auto root
#     -> switch-root -> systemd: sysusers, tmpfiles, homed, networkd ...
#
# What NixOS changes is where the operating system lives. In the pm tree it was
# the root partition, replaced wholesale by sysupdate. Here it is the Nix store,
# on a dm-verity protected /usr partition, and the root partition holds only
# state. That turns out to be the shape docs/factory-reset.md asked for and
# could not reach: see disk.nix.
{
  config,
  lib,
  pkgs,
  ...
}:

{
  boot = {
    # There is no bootloader to install. systemd-boot is copied into the ESP
    # when the image is assembled (image.nix), and nothing on the running
    # system ever rewrites it -- the same as an image-based system anywhere.
    loader.grub.enable = false;

    initrd.systemd = {
      enable = true;

      # Find the root filesystem by GPT partition type rather than by name.
      # systemd-gpt-auto-generator matches the type UUID, which is why there is
      # still no root= device on the command line and no fstab entry for /.
      root = "gpt-auto";
    };

    # The console is the only thing a VM test or a broken boot can be read
    # from; `quiet` keeps the kernel's own chatter off it on real hardware.
    kernelParams = [
      "quiet"
      "console=tty0"
    ];

    # Boot assessment is on for every UKI sysupdate installs (update.nix) and
    # deliberately off for the one the image ships with: the first kernel is
    # trusted by construction, and a counter on it would retire the only entry
    # there is.
    uki.tries = null;
  };

  # bless-boot marks a counted entry good once boot-complete.target is reached,
  # and boot-check-no-failures is what makes reaching it mean something. NixOS
  # carries bless-boot, and systemd-bless-boot-generator pulls it in whenever
  # the boot loader reports a boot counter, so it is no longer wanted by hand.
  # The check has to be named. Upstream's unit is RequiredBy= the target, and
  # NixOS ignores [Install] sections, so that is set here too.
  systemd.additionalUpstreamSystemUnits = [
    "systemd-boot-check-no-failures.service"
  ];
  systemd.services.systemd-boot-check-no-failures.requiredBy = [ "boot-complete.target" ];

  # The system is an image. It cannot be rebuilt in place, and it carries no
  # Nix: an update is a new /usr from systemd-sysupdate, not a switch.
  nix.enable = false;
  system.switch.enable = false;

  # /etc is an overlay of the generated tree rather than files written by an
  # activation script, which is what lets it be assembled without perl and
  # makes it read the same on every boot. Mutable, because networkd, homed and
  # hostnamed all keep state under /etc and the root partition is where state
  # lives here.
  system.etc.overlay = {
    enable = true;
    mutable = true;
  };

  # System users come from systemd-sysusers, reading sysusers.d generated from
  # users.users -- the upstream tool, not NixOS's perl script and not userborn.
  # Human users are not here at all: see accounts.nix.
  systemd.sysusers.enable = true;
  users.mutableUsers = false;
}
