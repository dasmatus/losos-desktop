# One network stack, and it is systemd's.
#
# networkd configures links, resolved answers DNS (with DNS-over-TLS and mDNS,
# so no avahi), timesyncd keeps the clock. That trade is carried over from the
# pm tree unchanged. Wi-Fi is wpa_supplicant's, the one daemon here that is not
# systemd's, because systemd has no supplicant; first-boot setup (setup.nix)
# joins a network through its control socket, the same way the installer
# does, and networkd runs DHCP on whatever it associates.
{ lib, ... }:

{
  networking = {
    useNetworkd = true;
    useDHCP = false;

    # GNOME turned NetworkManager on by default, and a desktop module still may. Two daemons fighting over the
    # same links is the one outcome worse than either alone.
    networkmanager.enable = lib.mkForce false;

    # systemd has no packet filter, so this is the one piece of networking that
    # is not systemd's. nftables rather than the iptables default, because it
    # is what networkd's own NFTSet= speaks, should anything want to use it.
    nftables.enable = true;

    wireless = {
      enable = true;
      # Control sockets under /run/wpa_supplicant for derisk to scan and join
      # through, and update_config=1, so a network joined there is written to
      # /etc/wpa_supplicant/imperative.conf and joined again after a reboot.
      # No networks are declared, so that file is wpa_supplicant's whole
      # configuration.
      userControlled = true;
    };
    firewall = {
      enable = true;
      # mDNS and LLMNR are answered by resolved, so it has to be reachable.
      allowedUDPPorts = [
        5353
        5355
      ];
    };
  };

  systemd.network = {
    enable = true;

    networks."20-wired" = {
      matchConfig = {
        Kind = "!*";
        Type = "ether";
      };
      networkConfig = {
        DHCP = "yes";
        MulticastDNS = true;
        LLMNR = true;
      };
      dhcpV4Config.UseDomains = true;
    };

    # Whatever station wpa_supplicant associates. A cable, when there is one,
    # carries the traffic: its default metric is lower.
    networks."30-wireless" = {
      matchConfig.WLANInterfaceType = "station";
      networkConfig = {
        DHCP = "yes";
        MulticastDNS = true;
        LLMNR = true;
      };
      dhcpV4Config = {
        UseDomains = true;
        RouteMetric = 600;
      };
      ipv6AcceptRAConfig.RouteMetric = 600;
    };

    # A desktop with its cable out should still boot to a login screen, and not
    # after a timeout: online means any link, not every link.
    wait-online.anyInterface = true;
  };

  services.resolved = {
    enable = true;
    settings.Resolve = {
      DNSOverTLS = "opportunistic";
      MulticastDNS = true;
      LLMNR = true;
    };
  };

  # timesyncd is NixOS's default already and is left as the default, not
  # restated: a VM sets it off at the same priority to use the host's clock.

  # resolved answers mDNS itself; avahi would be a second responder on 5353.
  services.avahi.enable = lib.mkForce false;
}
