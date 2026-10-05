# The web browser: Uranium, Chromium under the OS's own name, which turns
# into Chrome for Android on a phone (nixos/pkgs/uranium.nix, docs/nixos.md,
# "Uranium"). It is what opens a link or an HTML file unless the user picks
# something else.
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.losos.uranium;
  uranium = if cfg.patched then pkgs.uranium-patched else pkgs.uranium;
in
{
  options.losos.uranium.patched = lib.mkOption {
    type = lib.types.bool;
    default = false;
    description = ''
      Ship the Uranium compiled from Chromium's source with
      `nixos/pkgs/patches/chromium`, which says Uranium inside the browser
      too and on a phone sends Chrome for Android's client hints and lays
      pages out with Android's viewport. Off, Uranium wraps nixpkgs' own
      Chromium from cache.nixos.org with the same name, icon, profile and
      Android User-Agent string. Off by default because CI's runners cannot
      compile Chromium inside their time budget: turn it on where the build
      host can, or once the patched build is in the project's cache.
    '';
  };

  config = {
    environment.systemPackages = [ uranium ];

    # Chromium's policies, which Uranium reads from /etc/chromium as
    # Chromium does. Managed ones are fixed and the settings page shows them
    # greyed out; recommended ones are only the defaults, which the user can
    # change.
    #
    # Managed: everything that sends Google, or anyone, something the user
    # did not ask to send. ungoogled-chromium already compiles out Safe
    # Browsing, the Google API keys and the field trial configuration; these
    # name the rest, so the settings say so too and nothing comes back if a
    # later Chromium adds a path around a removed one.
    environment.etc."chromium/policies/managed/uranium.json".text = builtins.toJSON {
      # Usage statistics, crash reports, and the URLs of visited pages.
      MetricsReportingEnabled = false;
      UrlKeyedAnonymizedDataCollectionEnabled = false;
      # 2: no field trials, so no server decides which features are on.
      ChromeVariations = 2;
      FeedbackSurveysEnabled = false;
      UserFeedbackAllowed = false;
      # 0: no Safe Browsing, whose lists and lookups come from Google.
      SafeBrowsingProtectionLevel = 0;
      SafeBrowsingExtendedReportingEnabled = false;
      PasswordLeakDetectionEnabled = false;
      # Google's suggestions on an error page, its spelling service and its
      # translation, each of which sends the page's text or address.
      AlternateErrorPagesEnabled = false;
      SpellCheckServiceEnabled = false;
      TranslateEnabled = false;
      # 2: no preloading, which reaches servers for pages never opened.
      NetworkPredictionOptions = 2;
      DomainReliabilityAllowed = false;
      # Component updates fetch the certificate revocation list and the
      # like from Google, so they are off; the OS's updates bring a new
      # browser instead. So is asking Google's servers for the time.
      ComponentUpdatesEnabled = false;
      BrowserNetworkTimeQueriesEnabled = false;
      # Cast device discovery and its cloud services.
      EnableMediaRouter = false;
      WebRtcEventLogCollectionAllowed = false;
      WebRtcTextLogCollectionAllowed = false;
      # No Google account in the browser, so no sync to it either.
      BrowserSignin = 0;
      SyncDisabled = true;
      ShoppingListEnabled = false;
      PromotionsEnabled = false;
      # Google's AI features, which send the page to Google: 2 turns off
      # Help Me Write, tab comparison and history search, and 1 turns off
      # AI Mode in the address bar and the on-device model Chromium would
      # otherwise download for them. The Google Search side panel goes
      # with them.
      HelpMeWriteSettings = 2;
      TabCompareSettings = 2;
      HistorySearchSettings = 2;
      AIModeSettings = 1;
      GenAILocalFoundationalModelSettings = 1;
      GoogleSearchSidePanelEnabled = false;
      # No browser left running after its last window closes, which a
      # phone's battery pays for.
      BackgroundModeEnabled = false;
      # The OS sets the default browser (xdg.mime below), not a prompt.
      DefaultBrowserSettingEnabled = false;
    };

    # Recommended: defaults the user can change.
    environment.etc."chromium/policies/recommended/uranium.json".text = builtins.toJSON {
      # The address bar sends each keystroke to the search engine for its
      # suggestions; off until the user wants them.
      SearchSuggestEnabled = false;
    };

    # The program Uranium's tabs extension starts to list its tabs in
    # derisk's command palette (src/uranium-tabs). Chromium starts it only
    # for the extension this names: the id is the hash of the public key in
    # nixos/pkgs/uranium/tabs/manifest.json.
    environment.etc."chromium/native-messaging-hosts/org.losos.uranium_tabs.json".text =
      builtins.toJSON
        {
          name = "org.losos.uranium_tabs";
          description = "Lists Uranium's tabs in derisk's command palette";
          path = lib.getExe pkgs.uranium-tabs;
          type = "stdio";
          allowed_origins = [ "chrome-extension://cflkpneemglnjjmfpkbofdfenoidbcnn/" ];
        };

    # What a new profile starts with, which nixpkgs' build reads from here.
    # V8's optimizing compilers, Maglev and Turbofan, start off for every
    # site, as Edge's enhanced security mode has them: most of V8's
    # exploited bugs are in those compilers, and pages run on its
    # interpreter and baseline compiler, slower to compute but no less
    # complete. It is a preference and not a policy, which would lock it:
    # Settings → Privacy and security → Site settings → JavaScript
    # optimization turns them back on, for every site or for one.
    environment.etc."chromium/initial_preferences".text = builtins.toJSON {
      profile.default_content_setting_values.javascript_optimizer = 2;
    };

    # The default for web pages and links, which xdg-open, the portal's
    # OpenURI and derisk's own launcher all read from mimeapps.list.
    xdg.mime.defaultApplications = lib.genAttrs [
      "text/html"
      "application/xhtml+xml"
      "x-scheme-handler/http"
      "x-scheme-handler/https"
    ] (_: "uranium.desktop");
  };
}
