# What Uranium reads from /etc/chromium, or in the Flatpak from
# /app/chromium: its policies, its first profile's preferences and the
# native messaging host its tabs extension starts. Each attribute is a file
# under that directory, as the value JSON encodes. nixos/modules/browser.nix
# puts them in /etc, and uranium.nix in the Flatpak.
{
  # The program to start for Uranium's tabs extension.
  tabsHost,
}:

{
  # Chromium's policies. Managed ones are fixed and the settings page
  # shows them greyed out; recommended ones are only the defaults, which
  # the user can change.
  #
  # Managed: everything that sends Google, or anyone, something the user
  # did not ask to send. ungoogled-chromium already compiles out Safe
  # Browsing, the Google API keys and the field trial configuration; these
  # name the rest, so the settings say so too and nothing comes back if a
  # later Chromium adds a path around a removed one.
  "policies/managed/uranium.json" = {
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
  "policies/recommended/uranium.json" = {
    # The address bar sends each keystroke to the search engine for its
    # suggestions; off until the user wants them.
    SearchSuggestEnabled = false;
  };

  # The program Uranium's tabs extension starts to list its tabs in
  # derisk's command palette (src/uranium-tabs). Chromium starts it only
  # for the extension this names: the id is the hash of the public key in
  # nixos/pkgs/uranium/tabs/manifest.json.
  "native-messaging-hosts/org.losos.uranium_tabs.json" = {
    name = "org.losos.uranium_tabs";
    description = "Lists Uranium's tabs in derisk's command palette";
    path = tabsHost;
    type = "stdio";
    allowed_origins = [ "chrome-extension://cflkpneemglnjjmfpkbofdfenoidbcnn/" ];
  };

  # What a new profile starts with.
  # V8's optimizing compilers, Maglev and Turbofan, start off for every
  # site, as Edge's enhanced security mode has them: most of V8's
  # exploited bugs are in those compilers, and pages run on its
  # interpreter and baseline compiler, slower to compute but no less
  # complete. It is a preference and not a policy, which would lock it:
  # Settings → Privacy and security → Site settings → JavaScript
  # optimization turns them back on, for every site or for one.
  initial_preferences = {
    profile.default_content_setting_values.javascript_optimizer = 2;
  };
}
