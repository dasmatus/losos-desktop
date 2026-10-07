# System-wide ad and tracker blocking from EasyList-style filter lists
# (nixos/pkgs/losos-adblock.nix, docs/adblock.md).
#
# Two halves from the same lists. Every program's DNS goes through
# losos-adblock's forwarder, which answers 0.0.0.0 (or ::) for a blocked
# domain and passes everything else on: resolved sends every query to it
# as its only global server, and no link's own servers are a default
# route. That covers every app, Flatpaks and the Android layer included,
# but only whole domains. The rules that need a URL or a page (a path, a
# hidden element) are compiled into WebKit content blockers, which Danube
# loads into every page.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.losos.adblock;
  # Loopback, but not 127.0.0.53 or .54, which resolved listens on.
  address = "127.0.0.153";
in
{
  options.losos.adblock = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Block ads and trackers for every app, from `lists`.";
    };

    lists = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [
        "https://easylist.to/easylist/easylist.txt"
        "https://easylist.to/easylist/easyprivacy.txt"
        "https://adguardteam.github.io/AdGuardSDNSFilter/Filters/filter.txt"
        "https://pgl.yoyo.org/adservers/serverlist.php?hostformat=hosts&showintro=0&mimetype=plaintext"
      ];
      description = ''
        Filter lists, by https URL or absolute path: Adblock Plus syntax
        (EasyList, EasyPrivacy, AdGuard's), hosts files, or one domain per
        line. URLs are fetched daily. A user adds more, one per line, in
        /var/lib/losos-adblock/lists.
      '';
    };

    allow = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      example = [ "example.com" ];
      description = ''
        Domains never blocked, with their subdomains, whatever the lists
        say; in the browser, nothing is blocked on their pages either.
        /var/lib/losos-adblock/allow adds more.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ pkgs.losos-adblock ];

    environment.etc."losos-adblock/lists".text = lib.concatLines cfg.lists;
    environment.etc."losos-adblock/allow".text = lib.concatLines cfg.allow;

    # Its own user, static rather than DynamicUser, so the state directory
    # it owns can stay readable by everyone: Danube reads the compiled
    # WebKit rule sets from it.
    users.users.losos-adblock = {
      isSystemUser = true;
      group = "losos-adblock";
    };
    users.groups.losos-adblock = { };

    # resolved forwards every name to the forwarder, as its only global
    # server, and the links' own servers route nothing by themselves; the
    # forwarder reads them from /run/systemd/resolve/resolv.conf and sends
    # each query there. Queries for a link's own search domains still go
    # straight to that link's servers, as they must for a local network's
    # names. DNS over TLS cannot reach a forwarder on loopback that speaks
    # plain DNS, so it is off; the forwarder's own queries to the network's
    # servers are plain DNS too, which is what "opportunistic" fell back to
    # on most networks anyway.
    services.resolved.settings.Resolve = {
      DNS = address;
      Domains = "~.";
      DNSOverTLS = lib.mkForce "no";
    };
    systemd.network.networks = lib.genAttrs [ "20-wired" "30-wireless" ] (_: {
      networkConfig.DNSDefaultRoute = false;
    });

    systemd.sockets.losos-adblock = {
      description = "losos-adblock DNS forwarder";
      wantedBy = [ "sockets.target" ];
      before = [ "systemd-resolved.service" ];
      listenDatagrams = [ "${address}:53" ];
      listenStreams = [ "${address}:53" ];
      socketConfig.FreeBind = true;
    };

    systemd.services.losos-adblock = {
      description = "losos-adblock DNS forwarder";
      requires = [ "losos-adblock.socket" ];
      serviceConfig = {
        ExecStart = "${lib.getExe pkgs.losos-adblock} serve";
        User = "losos-adblock";
        Group = "losos-adblock";
        Restart = "always";
        # Only a reader of its blocklist and resolved's server list, and a
        # sender of DNS; port 53 is bound by systemd, so no capabilities.
        CapabilityBoundingSet = "";
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        PrivateDevices = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectControlGroups = true;
        RestrictAddressFamilies = [
          "AF_INET"
          "AF_INET6"
          "AF_UNIX"
        ];
        RestrictNamespaces = true;
        MemoryDenyWriteExecute = true;
        LockPersonality = true;
        SystemCallArchitectures = "native";
        SystemCallFilter = [ "@system-service" ];
      };
    };

    systemd.services.losos-adblock-update = {
      description = "Fetch and compile losos-adblock's filter lists";
      wants = [ "network-online.target" ];
      after = [ "network-online.target" ];
      serviceConfig = {
        Type = "oneshot";
        ExecStart = "${lib.getExe pkgs.losos-adblock} update";
        # 2 means a list could not be fetched; the rest were compiled and
        # the last good copy of that one kept, so it is not a failure.
        SuccessExitStatus = [ 2 ];
        User = "losos-adblock";
        Group = "losos-adblock";
        StateDirectory = "losos-adblock";
        StateDirectoryMode = "0755";
        UMask = "0022";
        CapabilityBoundingSet = "";
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        PrivateDevices = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectControlGroups = true;
        RestrictAddressFamilies = [
          "AF_INET"
          "AF_INET6"
          "AF_UNIX"
        ];
        RestrictNamespaces = true;
        LockPersonality = true;
        SystemCallArchitectures = "native";
        SystemCallFilter = [ "@system-service" ];
      };
    };

    # Shortly after boot, so a fresh install blocks within minutes, and
    # daily after; a boot that missed a day catches up.
    systemd.timers.losos-adblock-update = {
      wantedBy = [ "timers.target" ];
      timerConfig = {
        OnBootSec = "2min";
        OnCalendar = "daily";
        Persistent = true;
        # Spread the daily fetch, so the list hosts do not see every
        # machine at midnight.
        RandomizedDelaySec = "5min";
      };
    };

    # The state directory exists from the first boot, before the first
    # update, for the forwarder and Danube to read.
    systemd.tmpfiles.rules = [ "d /var/lib/losos-adblock 0755 losos-adblock losos-adblock -" ];
  };
}
