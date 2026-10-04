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
# exactly one fullscreen client, and so does the greeter. Drop cage from the
# greetd command once derisk can drive a TTY on its own.
{
  lib,
  pkgs,
  ...
}:

let
  # cage asks wlroots for a GL renderer, then Vulkan, and gives up rather than
  # draw in software. A GPU with no GL driver, such as QEMU's virtio-vga
  # without virgl, made every login end within a second and land back at the
  # login screen with no message. So a cage that exits at startup is run once
  # more on pixman, wlroots' CPU renderer: slow, but a screen. Ten seconds
  # separates that from a client that ran and ended, which a logout does by
  # terminating the logind session and never returns here. The greeter and
  # the session both run in cage, so both go through this; cage's -s keeps VT
  # switching, so a hung one can still be left for a console.
  derisk-cage = pkgs.writeShellScript "derisk-cage" ''
    start=$(${pkgs.coreutils}/bin/date +%s)
    ${lib.getExe pkgs.cage} -s -- "$@" && exit 0
    status=$?
    if [ -z "''${WLR_RENDERER:-}" ] && [ $(($(${pkgs.coreutils}/bin/date +%s) - start)) -lt 10 ]; then
      echo "derisk-cage: cage exited at startup ($status), retrying with WLR_RENDERER=pixman" >&2
      WLR_RENDERER=pixman exec ${lib.getExe pkgs.cage} -s -- "$@"
    fi
    exit "$status"
  '';

  derisk = lib.getExe pkgs.derisk;
in
{
  # The login screen is derisk's own lock screen, run as a greetd greeter
  # (`derisk greeter`), so logging in and unlocking look the same. greetd
  # replaced gdm: gdm is a GNOME Shell process with its own accounts daemon
  # and session chooser, all to offer one session, while greetd is a small
  # daemon that runs PAM and opens the logind session for whatever greeter it
  # is given. The greeter only relays the conversation; greetd's PAM service,
  # which NixOS generates with pam_systemd_home in it, is what unlocks a
  # homed user's home area at login. greetd runs the greeter as its own
  # unprivileged `greeter` user, and once the greeter has a user
  # authenticated and exits, starts the session command it was handed.
  #
  # --execute makes app launches and lock/suspend/reboot real rather than
  # simulated. There is one session, so no session chooser and no
  # wayland-sessions entry: the greeter is told the command outright.
  services.greetd = {
    enable = true;
    settings.default_session = {
      command = "${derisk-cage} ${derisk} greeter -- ${derisk-cage} ${derisk} session --execute";
      user = "greeter";
    };
  };

  # derisk-session.target, derisk-agent.socket and derisk-agent.service.
  systemd.packages = [ pkgs.derisk ];

  # Generate the PAM stack used by derisk's lock screen. With homed enabled,
  # NixOS includes pam_systemd_home in this service.
  security.pam.services.derisk = { };

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
