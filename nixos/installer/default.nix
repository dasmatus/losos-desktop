# The installer's live system: a kernel, systemd, wpa_supplicant and
# `derisk installer` as the session on tty1. installer.nix in the OS evaluates
# this and passes in, as `losos`, what to install and from where; iso.nix
# turns it into the ISO.
#
# derisk draws the pages and joins networks; `losos-installer serve`, its
# child, offers the disks and runs repart and sysupdate. Nothing else of the
# desktop is here: no display manager, no apps, no user. There is no
# NetworkManager: networkd runs DHCP on every wired port that has a cable and
# on whatever wireless link wpa_supplicant brings up, and derisk drives
# wpa_supplicant over its control socket, as first-boot setup does on the
# installed system. That is the whole network stack, and the only one an
# install needs.
{
  config,
  lib,
  pkgs,
  modulesPath,
  utils,
  losos,
  ...
}:

let
  systemd = config.systemd.package;

  # Where a release disk is mounted, when one is attached.
  releaseDisk = "/run/losos/release";

  # systemd-sysupdate is a libexec program, not one on PATH.
  sysupdateBin = pkgs.linkFarm "systemd-sysupdate-bin" [
    {
      name = "bin/systemd-sysupdate";
      path = "${systemd}/lib/systemd/systemd-sysupdate";
    }
  ];

  # Papirus without what nixpkgs propagates with it: breeze-icons, which
  # Papirus inherits from for KDE's icon names, and through it Qt. Neither
  # draws a single icon derisk asks for, and Qt alone would double the ISO.
  papirus = pkgs.papirus-icon-theme.overrideAttrs {
    propagatedBuildInputs = [ pkgs.hicolor-icon-theme ];
  };
in
{
  imports = [
    "${modulesPath}/profiles/minimal.nix"
    ./iso.nix
  ];

  system.stateVersion = "26.05";

  system.image = {
    id = "losos-installer";
    inherit (losos) version;
  };
  system.nixos = {
    distroId = "losos-installer";
    distroName = "LosOS Desktop installer";
  };

  boot = {
    # The medium boots its UKI directly (iso.nix); there is no boot loader to
    # configure and no disk of its own to put one on.
    loader.grub.enable = false;
    initrd.systemd.enable = true;

    # Errors only, so the kernel and systemd do not write over the console
    # before the installer takes the screen.
    kernelParams = [
      "quiet"
      "loglevel=3"
      "systemd.show_status=error"
    ];
  };

  # Every storage controller and USB host in the initrd, so the medium is found
  # wherever it is plugged in, and every network driver.
  hardware.enableAllHardware = true;

  # all-hardware.nix also turns on every redistributable firmware package, and
  # most of their bytes are audio DSP and other firmware an installer never
  # loads. linux-firmware alone holds what is needed: Wi-Fi, and the GPU
  # firmware without which amdgpu or nouveau would take the console over from
  # the EFI framebuffer and then fail to drive it.
  hardware.enableRedistributableFirmware = lib.mkForce false;
  hardware.firmware = [ pkgs.linux-firmware ];

  # Mesa, for derisk's GLES renderer and the GBM buffers it scans out, as on
  # the installed system (desktop.nix).
  hardware.graphics.enable = true;

  # The same choices as the OS (boot.nix): no Nix on the system, /etc as an
  # overlay, users from systemd-sysusers. Here they also keep perl out of a
  # system that never changes after it boots.
  nix.enable = false;
  system.switch.enable = false;
  system.etc.overlay = {
    enable = true;
    mutable = true;
  };
  systemd.sysusers.enable = true;
  users.mutableUsers = false;

  # tty2 and onwards log root in without a password, for wpa_cli and anything
  # else the installer does not cover. The machine is in front of whoever booted it,
  # and the installer that already runs on tty1 can erase any disk.
  services.getty.autologinUser = "root";
  users.allowNoPasswordLogin = true;
  systemd.services."getty@tty1".enable = false;
  systemd.services."autovt@tty1".enable = false;

  networking = {
    hostName = "losos-installer";
    useNetworkd = true;
    # The links are configured below, by name, rather than by nixpkgs'
    # catch-all defaults, so what a cable gets is written down here.
    useDHCP = false;

    wireless = {
      enable = true;
      # Control sockets under /run/wpa_supplicant, which derisk uses to scan
      # and to add the network someone picks.
      userControlled = true;
    };

    # Nothing listens: no SSH, and resolved is told below not to answer on
    # the local network. A packet filter would guard no port.
    firewall.enable = false;
  };

  systemd.network = {
    # Any physical wired port, built in or on USB: a cable that is plugged in,
    # at boot or later, gets an address with nothing asked, and the installer
    # goes straight on to the disks. Kind=!* leaves out bridges, bonds and the
    # like; wireless links have Type=wlan, so they are not matched here.
    networks."20-wired" = {
      matchConfig = {
        Type = "ether";
        Kind = "!*";
      };
      networkConfig.DHCP = "yes";
      # The lower metric wins, so with a cable and Wi-Fi both up the image
      # comes down the cable.
      dhcpV4Config.RouteMetric = 100;
      ipv6AcceptRAConfig.RouteMetric = 100;
    };

    # Whatever station wpa_supplicant associates, once someone picks a network
    # in the installer.
    networks."30-wireless" = {
      matchConfig.WLANInterfaceType = "station";
      networkConfig.DHCP = "yes";
      dhcpV4Config.RouteMetric = 600;
      ipv6AcceptRAConfig.RouteMetric = 600;
    };

    # Nothing here waits for the network: the installer watches for it, and a
    # machine with neither a cable nor Wi-Fi in range must still reach it.
    wait-online.enable = false;
  };

  services.resolved.settings.Resolve = {
    LLMNR = false;
    MulticastDNS = false;
  };

  # The release signing key, when the OS has one: sysupdate verifies
  # SHA256SUMS.gpg here exactly as it does on the installed system.
  environment.etc."systemd/import-pubring.gpg" = lib.mkIf (losos.pubring != null) {
    source = losos.pubring;
  };

  # A release disk: any filesystem labelled LOSOS-RELEASE holding a release's
  # files (SHA256SUMS, SHA256SUMS.gpg and what they list), which the installer
  # then installs from instead of the channel. It is for a machine that cannot
  # reach the channel, such as a VM behind a network that re-signs TLS with
  # its own authority. udev starts the mount when such a disk appears, at boot
  # or plugged in later, so a boot without one waits for nothing. A boot-time
  # mount with a device timeout gave up before udev had named a disk that was
  # there all along, and x-systemd.wanted-by= in fstab is not acted on for a
  # device unit.
  systemd.mounts = [
    {
      what = "/dev/disk/by-label/LOSOS-RELEASE";
      where = releaseDisk;
      type = "auto";
      options = "ro";
    }
  ];
  services.udev.extraRules = ''
    SUBSYSTEM=="block", ENV{ID_FS_LABEL}=="LOSOS-RELEASE", ENV{SYSTEMD_WANTS}+="${utils.escapeSystemdPath releaseDisk}.mount"
  '';

  # The installer is a logind session on tty1, as the login screen is on the
  # installed system, so derisk can take the display and input devices
  # through libseat. It runs as root: its backend partitions disks.
  #
  # This was `losos-installer` alone, a terminal interface on the same tty.
  # derisk draws the pages now, the same ones first-boot setup is made of,
  # so the installer works with a mouse, a touchscreen and a phone-sized
  # screen, and losos-installer is only the backend it starts.
  systemd.defaultUnit = "graphical.target";
  systemd.services.derisk-installer = {
    description = "LosOS Desktop installer";
    wantedBy = [ "graphical.target" ];
    after = [
      "systemd-user-sessions.service"
      "systemd-logind.service"
      "systemd-vconsole-setup.service"
    ];
    conflicts = [ "getty@tty1.service" ];

    path = [
      systemd
      sysupdateBin
      pkgs.coreutils
      pkgs.util-linux
      # repart formats the ESP and copies systemd-boot into it with these.
      pkgs.dosfstools
      pkgs.mtools
    ]
    # systemd-pull checks SHA256SUMS.gpg with gpg itself, and the installer
    # does the same for a release disk.
    ++ lib.optional (losos.pubring != null) pkgs.gnupg;

    environment = {
      # What pam_systemd records as the session's type.
      XDG_SESSION_TYPE = "wayland";
      # The icon theme derisk's built-in themes name, for the keyboard
      # toggle and the page icons. Named directly: minimal.nix leaves icons
      # out of the system profile, and a service has no profile anyway.
      XDG_DATA_DIRS = "${papirus}/share";
    }
    # Where an error alert's section is published. The alert prints the
    # address for reading on another device: the live system has no browser
    # (docs.nix).
    // lib.optionalAttrs (losos.docsUrl != null) {
      MCSAPI_DOCS_URL = losos.docsUrl;
    };

    serviceConfig = {
      ExecStart = lib.escapeShellArgs [
        (lib.getExe pkgs.derisk)
        "installer"
        "--"
        (lib.getExe pkgs.losos-installer)
        "serve"
        "--name"
        "LosOS Desktop"
        "--repart-definitions"
        losos.repartDefinitions
        "--sysupdate-definitions"
        losos.sysupdateTemplates
        "--esp"
        losos.esp
        "--medium"
        "/iso"
        "--work"
        "/run/losos-installer"
        "--source"
        losos.baseUrl
        "--local-source"
        releaseDisk
      ];
      PAMName = "derisk-installer";
      User = "root";
      TTYPath = "/dev/tty1";
      TTYReset = true;
      TTYVHangup = true;
      TTYVTDisallocate = true;
      UtmpIdentifier = "tty1";
      UtmpMode = "user";
      StandardInput = "tty-fail";
      # derisk and the tools its backend runs log to the journal; the log
      # pane on the last page shows the tools' output from the backend.
      StandardOutput = "journal";
      StandardError = "journal";
      # A crash is still a crash; the next person to look at the screen
      # should find the installer there.
      Restart = "always";
      RestartSec = 1;
    };
  };

  security.pam.services.derisk-installer.startSession = true;
}
