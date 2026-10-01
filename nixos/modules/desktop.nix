# GNOME, as a tree of systemd user units.
#
# gnome-session in nixpkgs runs the session as systemd user units under
# graphical-session.target rather than supervising its own processes, which is
# what the pm tree built with -Dsystemd_session=enabled. So `systemctl --user
# status` describes the desktop, a crashed component is restarted by the same
# supervisor as everything else, and systemd-oomd can act on one
# application's cgroup instead of the kernel OOM killer taking the compositor.
# mutter and gdm get their DRM and input devices from logind, which is what
# lets the compositor run without root.
{ lib, pkgs, ... }:

{
  services.displayManager.gdm.enable = true;
  services.desktopManager.gnome.enable = true;

  # The pm tree built a deliberately small GNOME: shell, settings, files. The
  # core app set is left out for the same reason, with Files and a terminal
  # put back because a desktop without them cannot be used to do anything.
  services.gnome.core-apps.enable = false;
  environment.systemPackages = with pkgs; [
    nautilus
    ptyxis
  ];

  # Let systemd-oomd act on a desktop session before the kernel OOM killer
  # does. oomd watches cgroup pressure and kills the worst-behaved application
  # cgroup while the machine is still responsive, but only in slices that opt
  # in; this opts in every user's. NixOS sets the limit at 80% pressure on
  # user.slice. 50% is Fedora's for a desktop, where a session stalling for
  # half of every interval is already unusable.
  systemd.oomd = {
    enable = true;
    enableUserSlices = true;
  };
  systemd.slices.user.sliceConfig.ManagedOOMMemoryPressureLimit = "50%";

  # Firmware updates. fwupd itself is bus-activated; the metadata refresh
  # timer is what makes updates appear without anyone asking.
  services.fwupd.enable = true;

  # Mesa, for the compositor.
  hardware.graphics.enable = true;

  # Nothing is needed for FIDO2. The pm tree had to add CONFIG_HIDRAW to its
  # kernel fragment because arm64's defconfig leaves it off; nixpkgs'
  # common-config.nix sets it for every architecture, and systemd's fido_id and
  # uaccess rules do the rest. `homectl update --fido2-device=auto` works as-is.
}
