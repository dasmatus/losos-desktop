# The installer's live system: a kernel, systemd, wpa_supplicant and the
# installer TUI on tty1. installer.nix in the OS evaluates this and passes in,
# as `losos`, what to install and from where; iso.nix turns it into the ISO.
#
# Nothing of the desktop is here. There is no NetworkManager: networkd runs
# DHCP on every wired port that has a cable and on whatever wireless link
# wpa_supplicant brings up, and the TUI drives wpa_supplicant over its control
# socket. That is the
# whole network stack, and the only one an install needs.
{
  config,
  lib,
  pkgs,
  modulesPath,
  losos,
  ...
}:

let
  systemd = config.systemd.package;

  # systemd-sysupdate is a libexec program, not one on PATH.
  sysupdateBin = pkgs.linkFarm "systemd-sysupdate-bin" [
    {
      name = "bin/systemd-sysupdate";
      path = "${systemd}/lib/systemd/systemd-sysupdate";
    }
  ];
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

    # Errors only, so the kernel and systemd do not write over the TUI on the
    # same console.
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
  # else the TUI does not cover. The machine is in front of whoever booted it,
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
      # Control sockets under /run/wpa_supplicant, which the TUI uses to scan
      # and to add the network someone picks.
      userControlled = true;
    };

    # Nothing listens: no SSH, and resolved is told below not to answer on
    # the local network. A packet filter would guard no port.
    firewall.enable = false;
  };

  systemd.network = {
    # Any physical wired port, built in or on USB: a cable that is plugged in,
    # at boot or later, gets an address with nothing asked, and the TUI goes
    # straight on to the disks. Kind=!* leaves out bridges, bonds and the
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
    # in the TUI.
    networks."30-wireless" = {
      matchConfig.WLANInterfaceType = "station";
      networkConfig.DHCP = "yes";
      dhcpV4Config.RouteMetric = 600;
      ipv6AcceptRAConfig.RouteMetric = 600;
    };

    # Nothing here waits for the network: the TUI watches for it, and a
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

  systemd.services.losos-installer = {
    description = "LosOS Desktop installer";
    wantedBy = [ "multi-user.target" ];
    after = [
      "systemd-user-sessions.service"
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
    # systemd-pull checks SHA256SUMS.gpg with gpg itself.
    ++ lib.optional (losos.pubring != null) pkgs.gnupg;

    environment.TERM = "linux";

    serviceConfig = {
      ExecStart = lib.escapeShellArgs [
        (lib.getExe pkgs.losos-installer)
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
      ];
      StandardInput = "tty";
      StandardOutput = "tty";
      # The TUI owns the screen. What the tools it runs print reaches its log
      # pane through pipes; anything else goes to the journal.
      StandardError = "journal";
      TTYPath = "/dev/tty1";
      TTYReset = true;
      TTYVHangup = true;
      TTYVTDisallocate = true;
      # Ctrl+C quits, and a crash is still a crash; either way the next
      # person to look at tty1 should find the installer there.
      Restart = "always";
      RestartSec = 1;
    };
  };
}
