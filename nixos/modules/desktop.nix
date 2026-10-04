# derisk, as a tree of systemd user units.
#
# derisk (github.com/dasmatus/derisk) replaces GNOME. It launches every app as
# a transient unit in app-graphical.slice, binds its own derisk-session.target
# to graphical-session.target, and serves its agent socket from a
# socket-activated user unit, so `systemctl --user status` still describes
# the desktop and systemd-oomd can still act on one application's cgroup.
#
# derisk has no DRM/KMS backend yet: `derisk session` only runs nested inside
# another Wayland or X11 session. Until it grows one, the session runs it
# inside cage, a kiosk compositor that owns the seat through logind and shows
# exactly one fullscreen client. Drop cage from the session's Exec= once
# derisk can drive a TTY on its own.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  # One wayland-sessions entry, so gdm offers derisk and nothing else.
  # --execute makes app launches and lock/suspend/reboot real rather than
  # simulated. cage's -s keeps VT switching, so a hung session can still be
  # left for a console.
  derisk-session =
    (pkgs.writeTextDir "share/wayland-sessions/derisk.desktop" ''
      [Desktop Entry]
      Name=derisk
      Comment=Adaptive, agent-first Wayland desktop
      Exec=${lib.getExe pkgs.cage} -s -- ${lib.getExe pkgs.derisk} session --execute
      Type=Application
      DesktopNames=derisk
    '').overrideAttrs
      { passthru.providedSessions = [ "derisk" ]; };
in
{
  # gdm stays as the login screen: it is what sees homed users through NSS
  # (accounts.nix) and what the VM test boots to. It is a display manager, not
  # the desktop, and it does not need GNOME's desktop module.
  services.displayManager = {
    gdm.enable = true;
    sessionPackages = [ derisk-session ];
    defaultSession = "derisk";
  };

  # derisk-session.target, derisk-agent.socket and derisk-agent.service.
  systemd.packages = [ pkgs.derisk ];

  # Do not implement pre-sleep locking from a system oneshot: `loginctl
  # lock-sessions` only emits logind's Lock signal and returns immediately, so
  # ordering this Before=sleep.target does not guarantee the lock screen is
  # actually active before suspend.
  #
  # Reliable pre-sleep locking must be implemented by the session compositor
  # itself using a logind `sleep` delay inhibitor, locking, and only then
  # releasing the inhibitor.

  # derisk's Files, Settings, Text Editor, System Monitor and Calculator are
  # built into the derisk binary, which replaces GNOME's Files. A terminal is
  # the one app it does not have, and foot is the one its own docs launch.
  environment.systemPackages = [
    pkgs.derisk
    pkgs.foot
  ];

  # What GNOME's module used to switch on, kept where something in the session
  # still uses it. Audio for every app. udisks2 and upower are bus-activated
  # and back removable drives and the overview's battery widget. The keyring
  # is the Secret Service every app that stores a password asks for. The GTK
  # portal gives sandboxed and GTK apps a file chooser, since derisk ships no
  # portal of its own.
  #
  # Left off, because nothing in derisk has a UI for them yet: Bluetooth,
  # power profiles and geoclue.
  services.pipewire = {
    enable = true;
    pulse.enable = true;
  };
  services.udisks2.enable = true;
  services.upower.enable = true;
  services.gnome.gnome-keyring.enable = true;
  xdg.portal = {
    enable = true;
    extraPortals = [ pkgs.xdg-desktop-portal-gtk ];
    config.derisk.default = [ "gtk" ];
  };

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

  # Mesa, for cage and for derisk's GLES renderer.
  hardware.graphics.enable = true;

  # Nothing is needed for FIDO2. The pm tree had to add CONFIG_HIDRAW to its
  # kernel fragment because arm64's defconfig leaves it off; nixpkgs'
  # common-config.nix sets it for every architecture, and systemd's fido_id and
  # uaccess rules do the rest. `homectl update --fido2-device=auto` works as-is.
}
