# First-boot setup: derisk's own pages, on the seat, before the login screen.
#
# A fresh install has no user, and homed's console wizard used to make the
# first one. `derisk setup` does that job, and also asks what that wizard
# never did: a language, a keyboard layout, a time zone and a network. It
# saves them through localed, timedated and homed, which is where the rest of
# the system reads them from, and exits. The display manager then takes the
# seat with that one user to log in as.
#
# It runs on every boot and exits at once when a regular user exists, rather
# than only on systemd's first boot: a machine switched off halfway through
# must ask again, not reach a login screen with nobody on it.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  systemd = config.systemd.package;

  # homed's console wizard, kept as the fallback for a machine derisk cannot
  # draw on (no GPU driver, no KMS). It skips itself when the setup made a
  # user, so it only ever asks after a failure.
  consoleWizard = lib.escapeShellArgs [
    "${systemd}/bin/homectl"
    "firstboot"
    "--prompt-new-user"
    "--prompt-shell=no"
    "--prompt-groups=yes"
  ];

  setup = pkgs.writeShellScript "derisk-setup" ''
    ${lib.getExe pkgs.derisk} setup || exec ${consoleWizard}
  '';
in
{
  # homed's first-boot unit stays, for what it does besides asking: making
  # users from `home.create.*` credentials, as a provisioning tool or a VM
  # passes them. The asking is the setup's.
  services.homed.promptOnFirstBoot = true;
  systemd.services.systemd-homed-firstboot.serviceConfig.ExecStart = [
    ""
    "${systemd}/bin/homectl firstboot"
  ];

  # localectl and timedatectl write /etc/locale.conf, localed's keyboard file
  # and /etc/localtime; with these declared, NixOS would put its own links
  # there and the choices would not stick. `time.timeZone` is left unset for
  # the same reason.
  i18n.imperativeLocale = true;

  # The languages the setup offers are the locales installed, so every one it
  # has a name for (derisk's locale.rs) is built into the locale archive.
  # glibc's whole set is ten times the size, for languages derisk's pages
  # would show by their codes.
  i18n.extraLocales = map (l: "${l}.UTF-8/UTF-8") [
    "cs_CZ"
    "da_DK"
    "de_AT"
    "de_CH"
    "de_DE"
    "el_GR"
    "en_AU"
    "en_CA"
    "en_GB"
    "en_IE"
    "en_IN"
    "en_US"
    "es_ES"
    "es_MX"
    "fi_FI"
    "fr_CA"
    "fr_FR"
    "hu_HU"
    "it_IT"
    "ja_JP"
    "ko_KR"
    "nb_NO"
    "nl_NL"
    "pl_PL"
    "pt_BR"
    "pt_PT"
    "ro_RO"
    "ru_RU"
    "sk_SK"
    "sv_SE"
    "tr_TR"
    "uk_UA"
    "zh_CN"
    "zh_TW"
  ];

  systemd.services.derisk-setup = {
    description = "First-boot setup";
    wantedBy = [ "graphical.target" ];
    # The login screen waits for it, so the two never fight over the seat,
    # and it waits for homed, so it can tell whether a user exists.
    before = [ "derisk-display-manager.service" ];
    after = [
      "systemd-user-sessions.service"
      "systemd-logind.service"
      "systemd-homed.service"
      "systemd-homed-firstboot.service"
      "getty@tty1.service"
    ];
    wants = [ "systemd-homed.service" ];
    conflicts = [ "getty@tty1.service" ];

    # userdbctl, localectl, timedatectl and homectl; and glibc's `locale`,
    # which lists the locales in the archive LOCALE_ARCHIVE names, where
    # localectl only looks under /usr/lib/locale.
    path = [
      systemd
      pkgs.glibc.bin
    ];

    environment = {
      # What pam_systemd records as the session's type.
      XDG_SESSION_TYPE = "wayland";
      # The layouts the Keyboard page lists, and the icon theme its buttons
      # are drawn from: a service has no profile to set either.
      XKB_CONFIG_ROOT = "${pkgs.xkeyboard_config}/share/X11/xkb";
      XDG_DATA_DIRS = "/run/current-system/sw/share";
    };

    serviceConfig = {
      Type = "oneshot";
      ExecStart = setup;
      # A logind session on tty1 is what lets derisk take the display and
      # input devices through libseat, as the greeter does.
      PAMName = "derisk-setup";
      User = "root";
      TTYPath = "/dev/tty1";
      TTYReset = true;
      TTYVHangup = true;
      TTYVTDisallocate = true;
      UtmpIdentifier = "tty1";
      UtmpMode = "user";
      # The console wizard reads and writes the terminal; derisk logs to the
      # journal itself.
      StandardInput = "tty-fail";
      StandardOutput = "tty";
      StandardError = "journal";
      # However long a person takes to type a name.
      TimeoutStartSec = "infinity";
    };
  };

  security.pam.services.derisk-setup.startSession = true;
}
