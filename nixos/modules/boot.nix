# How it boots. Every stage is systemd, as in docs/boot.md:
#
#   firmware -> systemd-boot -> UKI (systemd-stub) -> kernel
#     (a legacy BIOS: MBR -> GRUB -> the UKI's kernel and initrd, bios.nix)
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
    # The same goes for GRUB on a BIOS PC (bios.nix), so NixOS's own GRUB
    # module, which installs GRUB and rewrites its menu on every rebuild,
    # stays off.
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

  # nix.enable, the /etc overlay and sysusers are in base.nix: a Halium build
  # is the same kind of image and shares them.
}
