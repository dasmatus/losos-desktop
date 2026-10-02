# One network stack, and it is systemd's.
#
# networkd configures links, resolved answers DNS (with DNS-over-TLS and mDNS,
# so no avahi), timesyncd keeps the clock. GNOME's network panel talks to
# NetworkManager and is therefore inert here; that trade is carried over from
# the pm tree unchanged, and so is the cost that there is no Wi-Fi
# configuration UI.
{ lib, ... }:

{
  networking = {
    useNetworkd = true;
    useDHCP = false;

    # GNOME turns NetworkManager on by default. Two daemons fighting over the
    # same links is the one outcome worse than either alone.
    networkmanager.enable = lib.mkForce false;

    # systemd has no packet filter, so this is the one piece of networking that
    # is not systemd's. nftables rather than the iptables default, because it
    # is what networkd's own NFTSet= speaks, should anything want to use it.
    nftables.enable = true;
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
