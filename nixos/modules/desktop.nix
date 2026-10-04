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
# display manager's command once derisk can drive a TTY on its own.
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
  # derisk is the display manager too: `derisk display-manager` runs as root
  # on VT1 and does only PAM and session starts, drawing nothing. It runs
  # derisk's lock screen as the login screen (`derisk greeter`), so logging in
  # and unlocking are the same screen, and the greeter runs as its own
  # unprivileged user and talks to it over a socket only that user can open.
  # After a login it opens the user's session through the derisk-login PAM
  # service and runs the session command as them; when the session ends the
  # greeter comes back.
  #
  # This replaced gdm, a GNOME Shell process with its own accounts daemon and
  # session chooser, all to offer one session. greetd would also have done:
  # the greeter speaks its protocol and runs under it unchanged. But it is one
  # more daemon between derisk and PAM doing the same small job, and without
  # it nothing on the login path is anything but derisk and systemd.
  #
  # --execute makes app launches and lock/suspend/reboot real rather than
  # simulated. There is one session, so no session chooser and no
  # wayland-sessions entry: the greeter is handed the command outright.
  systemd.services.derisk-display-manager = {
    description = "derisk display manager";
    aliases = [ "display-manager.service" ];
    wantedBy = [ "graphical.target" ];
    # VT1 is the display manager's, as it was gdm's: no getty there.
    conflicts = [ "getty@tty1.service" ];
    after = [
      "systemd-user-sessions.service"
      "getty@tty1.service"
      "systemd-logind.service"
    ];
    wants = [ "systemd-user-sessions.service" ];
    serviceConfig = {
      ExecStart = lib.escapeShellArgs [
        derisk
        "display-manager"
        "--vt"
        "1"
        "--"
        derisk-cage
        derisk
        "greeter"
        "--"
        derisk-cage
        derisk
        "session"
        "--execute"
      ];
      Type = "notify";
      Restart = "always";
      # Sessions live in their own logind scopes; stopping the display
      # manager should not take a logged-in desktop down with it.
      KillMode = "process";
      # Its own runtime directory, for the greeter's socket.
      RuntimeDirectory = "derisk-dm";
      RuntimeDirectoryMode = "0711";
    };
  };

  # The greeter's user: a system user with no home and no login, whose only
  # power is asking the display manager to check a password.
  users.users.derisk-greeter = {
    isSystemUser = true;
    group = "derisk-greeter";
    description = "derisk login screen";
  };
  users.groups.derisk-greeter = { };

  # derisk-login is a full login: startSession puts pam_systemd (the logind
  # session) in it, and homed adds pam_systemd_home. derisk-greeter is the
  # greeter's own session, which only needs pam_systemd, for the logind
  # session cage opens the display through. Its auth stack goes unused: the
  # display manager never authenticates the greeter or sets its credentials.
  security.pam.services.derisk-login.startSession = true;
  security.pam.services.derisk-greeter.startSession = true;

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
