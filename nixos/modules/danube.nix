# The web browser: Danube, WPE WebKit drawn inside an mcsapi window
# (nixos/pkgs/danube.nix, docs/danube.md). It is what opens a link or an
# HTML file unless the user picks something else, and the window that
# opens when a network wants a sign-in first (a captive portal).
#
# Uranium, Chromium under the OS's name with a Flatpak build, was here
# before; Danube replaced it, and its options went with it
# (docs/danube.md, "What it replaced").
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.losos.danube;
  portal = config.losos.captivePortal;
  settings = lib.generators.toKeyValue { } (
    {
      "javascript.jit" = lib.boolToString cfg.jit;
    }
    // lib.optionalAttrs (cfg.home != null) { home = cfg.home; }
  );
in
{
  options.losos.danube = {
    home = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "https://start.example/";
      description = ''
        What a new tab opens. Null keeps Danube's default, an empty page.
        There is no `search` option: Danube has no address bar, derisk's
        command palette is it, and the palette searches with the engine
        chosen in Settings, Default apps (docs/danube.md, "The window").
      '';
    };

    jit = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Let JavaScriptCore compile pages' scripts to machine code. Off,
        scripts and WebAssembly run in its interpreters: slower, and a page
        can no longer have the browser write and run code, which is what
        most WebKit exploits rely on. A user turns it back on with
        `javascript.jit = true` in ~/.config/danube/settings.conf.
      '';
    };
  };

  options.losos.captivePortal = {
    enable = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Check for a captive portal whenever the network changes, and open
        Danube's sign-in window when one is in the way.
      '';
    };

    checkUrl = lib.mkOption {
      type = lib.types.str;
      default = "http://nmcheck.gnome.org/check_network_status.txt";
      description = ''
        A plain-HTTP URL that answers `expect` (or 204 with no body, when
        `expect` is empty) when the network lets traffic through. A portal
        intercepts it and answers something else. GNOME's, by default, as
        NetworkManager's GNOME setup uses; each check sends the request
        line, the host and "Danube" as the User-Agent, nothing else.
      '';
    };

    expect = lib.mkOption {
      type = lib.types.str;
      default = "NetworkManager is online";
      description = "The body `checkUrl` answers with when online.";
    };
  };

  config = {
    environment.systemPackages = [ pkgs.danube ];

    environment.etc."danube/settings.conf".text = settings;
    environment.etc."danube/captive-portal.conf".text = ''
      url = ${portal.checkUrl}
      expect = ${portal.expect}
    '';

    # One per session: it opens a window, so it runs as the person signed
    # in, and only while their desktop does.
    systemd.user.services.danube-captive-watch = lib.mkIf portal.enable {
      description = "Open Danube's sign-in window for a captive portal";
      wantedBy = [ "graphical-session.target" ];
      partOf = [ "graphical-session.target" ];
      after = [ "graphical-session.target" ];
      serviceConfig = {
        ExecStart = "${lib.getExe pkgs.danube} captive-watch";
        Restart = "on-failure";
      };
    };

    # The default for web pages and links, which xdg-open, the portal's
    # OpenURI and derisk's own launcher all read from mimeapps.list.
    xdg.mime.defaultApplications = lib.genAttrs [
      "text/html"
      "application/xhtml+xml"
      "x-scheme-handler/http"
      "x-scheme-handler/https"
    ] (_: "org.losos.Danube.desktop");
  };
}
