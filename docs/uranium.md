# Uranium

Uranium is the OS's web browser and the default for links and web pages
(`nixos/modules/browser.nix`). It is Chromium under the OS's own name, and
on a phone it becomes Chrome for Android as far as the web can tell: it
sends Chrome for Android's User-Agent string, so sites serve their Android
version, and lays the page out as Chrome for Android does. A phone is the
same line as in [GTK and Qt on a phone](gtk-qt-phone.md), every screen under 600 logical pixels on its short
side; the launcher measures it once, at launch, over xdg-output, and
`URANIUM_FORM_FACTOR=phone` or `desktop` overrides it. On a phone it also
turns on Chromium's touch layout for the tab strip and toolbar, overlay
scrollbars and touch events, and Chrome for Android's power saving, which
the desktop build carries but leaves off: hidden pages are frozen after a
grace period and give back their memory, and every background tab but the
last one used can be frozen. Everywhere it uses Wayland and
`text-input-unstable-v3`, so derisk's on-screen keyboard follows its text
fields; Chromium speaks v1 unless told.

Both start from nixpkgs' ungoogled-chromium rather than its Chromium:
ungoogled-chromium's patches take out the code that talks to Google
(sign-in, sync, Safe Browsing's lookups, the RLZ and field-trial pings,
the Web Store's update checks) and replace Google's domains in what is
left with ones that cannot resolve.

There are two builds of it (`nixos/pkgs/uranium.nix`):

- **`uranium-patched`** compiles Chromium from nixpkgs' source with
  `nixos/pkgs/patches/chromium` and `nixos/pkgs/uranium/rebrand.py`,
  after nixpkgs' patches and before ungoogled-chromium's. The
  patches make two upstream switches mean Android on Linux, where
  upstream reads them only on Android: `--use-mobile-user-agent` sends the
  Android User-Agent string and client hints (`Sec-CH-UA-Mobile: ?1`,
  `Sec-CH-UA-Platform: "Android"`), and `--enable-viewport` turns on all of
  WebPreferences' Android viewport settings, as DevTools' phone emulation
  does, where upstream turned on only the 980 pixel layout viewport and
  still ignored the page's viewport tag. A third patch keeps the profile in
  `~/.config/uranium` and names the desktop file and icon. The fourth is
  the phone interface, behind `--uranium-phone-ui`: Chrome for Android's
  own interface is Java written against Android's views and cannot be built
  on Linux, so this gives the desktop interface its shape instead. The
  toolbar is the only row, with no tab strip; a tab switcher button beside
  the menu opens Tab Search's list of open tabs; the forward, extensions
  and profile buttons are gone. The same patch adds
  `--uranium-single-view`, which the launcher always passes to this
  build: every window is one page, with the address bar but no tab strip,
  and its tabs are reached through derisk's command palette (below). Kiosk
  mode was not used for this because it also takes away the address bar,
  the menu and the back button. The fifth patch puts the toolbar, with the
  address bar, at the bottom of the window on a phone, and on every screen
  when Settings, Appearance, "Show the address bar at the bottom" is on. Nobody has compiled it yet, and Tab
  Search's bubble still anchors to the hidden tab strip's button, so where
  it opens is the first thing a real build has to check. `rebrand.py`
  renames Chromium to Uranium in the roughly 600 interface strings that say
  it, in English and in all 81 translations, keeping ChromiumOS and the
  Chromium Authors as they are. It has GRIT, from the same source tree,
  re-hash each renamed message and moves the translations to the new ids;
  renaming the text alone would leave every locale showing those strings in
  English.
- **`uranium`**, the one the image ships, wraps nixpkgs' own
  ungoogled-chromium build, which substitutes from cache.nixos.org, in the same launcher,
  desktop file, icon and profile directory. Without the patches it can send
  only the Android User-Agent string, not the client hints, it keeps the
  desktop viewport, the browser's own windows still say Chromium, and its
  windows keep their tab strip.
  Its Chromium comes from nixpkgs without this repository's overlay: from
  the overlay's package set it would link the patched GTK and need
  compiling as well.

What the browser may do is set as Chromium policy, in
`nixos/pkgs/uranium/config.nix`, which both builds read from
`/etc/chromium` and the Flatpak from `/app/chromium`. Each policy there
carries its reason; together they switch off what ungoogled-chromium's
patches leave reaching out: metrics, the variations seed, Safe Browsing,
search suggestions, network prediction, Domain Reliability, the time
queries, component updates, the media router, translation, the spelling
service, sign-in, sync and the AI features. The launcher adds
`--no-pings` (no `<a ping>` beacons) and
`--disable-background-networking`. V8's optimizing compilers are off by
default through the first-run preference for the JavaScript optimizer, the
same switch as Settings, Site settings, "V8 optimizer"; a site the user trusts
can have them back there. JavaScript itself stays on, because most of the
web does not work without it.

Every profile starts with uBlock Origin Lite, the Manifest V3 uBlock
Origin, which blocks through Chromium's own declarativeNetRequest engine.
Its settings choose its filter lists: ads, trackers, annoyances, malware
domains and each region's own. Chromium writes the lists' compiled rules
into the extension's own directory, so the launcher copies it from the
store to `~/.local/share/uranium/extensions` on each launch, and its
manifest carries a fixed key so the user's choice of lists outlives an
update. The Web Store is unreachable from an ungoogled build, so the
extension updates with the OS.

Uranium's interface is drawn in mcsapi's look, as every app derisk shows
is. On each launch the launcher reads the theme id derisk publishes in
`$XDG_RUNTIME_DIR/derisk/theme.json` and runs mcsapi's `x2mcsapi`
(`nixos/pkgs/x2mcsapi.nix`, the mcsapi commit derisk builds against),
which writes a GTK theme generated from that mcsapi theme; the launcher
selects it with `GTK_THEME`, and new profiles start on Chromium's GTK
theme, so the toolbar, tabs, menus and dialogs take its colours. A theme
changed while Uranium runs shows from its next launch. Web pages are left
as their authors made them: restyling them breaks sites.

Tabs show up in derisk's command palette and top bar as each window's
global menus, "Tabs" (switch to one, or open a new one) and "Close Tab".
Chromium exports no menus on Wayland, so a second built-in extension
sends each window's tabs, by native messaging, to `uranium-tabs`
(`src/uranium-tabs`), which registers them on derisk's agent socket and
hands a pick back to the extension. derisk delivers the pick to the
connection that registered the menu (derisk#35). Windows pair by their
title, which Chromium takes from the active tab.

The patched build is compiled by `.github/workflows/uranium.yml`, in
rounds: Chromium takes longer than one runner's six hours. Each round
restores a ccache in GHCR (`uranium-ccache:<arch>`), builds for nearly
five hours, saves the cache, pushes what it finished to the project's Nix
cache, and dispatches the next round for the architectures still going,
up to eight. The compiler wrapper uses the cache only where the builder
binds one into the sandbox, so the derivation is the same with or
without it. Chromium's ThinLTO cache, which keeps each module's optimized
code across links, goes in the same directory, so the final link of a
round redoes only the modules that changed since the last one. The build sets ThinLTO on both architectures and CFI
(`is_cfi`, `use_cfi_icall`) on x86_64, and fails if gn drops any of them;
an official build turns those on anyway, so this guards them rather than
adding them. Chromium does not build CFI for arm64 Linux, where PAC and
BTI cover what the CPU supports. Uranium is the one thing this repository
builds that does not link with mold ([glibc and stock packages](packages.md)):
Chromium accepts ThinLTO, and so CFI, only with lld. ci.yml still checks, as
`checks.<system>.uranium-patches`, that every patch applies, ungoogled's
included, and the rename still finds every string, against the source
nixpkgs pins. `losos.uranium.patched = true` puts the patched build in the
image once the project cache holds it.

The same workflow packs Uranium as the Flatpak `org.losos.Uranium`
(`nixos/pkgs/uranium/flatpak`) on Flathub's Chromium base app, and on
main pushes it to GHCR as `flatpak:uranium-<arch>`. A Flatpak cannot use
a Chromium built into the Nix store, so its Chromium is
ungoogled-chromium's portable build of the same version, unpatched; the
launcher, both extensions, policies and helpers are the same Nix
expressions as the image's. The proxy serves it as an OCI Flatpak remote
at `/flatpak/`: an index listing each architecture's image by digest,
and the registry calls Flatpak makes for them. The flake sets
`losos.uranium.flatpakRemote` from `LOSOS_PROXY_URL`, and the OS adds it
as the remote `losos`, beside Flathub. `losos.uranium.flatpak = true`
installs and updates Uranium from there at boot instead of shipping it in
the image.

The other reading of "the Android version" is the real Chrome for Android
APK, run through the Android Translation Layer (`losos.android.enable`).
That is not done: ATL does not run yet (its package is still marked
broken), and a browser would be the largest app it had been asked to run.
