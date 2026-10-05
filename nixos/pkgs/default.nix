# The three programs this repository writes rather than fetches, pm, the
# system manager, and the Halium hardware layer nixpkgs does not carry, plus
# GTK and Qt with the patches that make their applications fit a phone.
# Everything else the OS runs comes from nixpkgs unchanged.
final: prev:
let
  # A directory of patches, applied in file name order, which the numeric
  # prefixes make the order they were written to apply in.
  patchesIn =
    dir:
    map (name: dir + "/${name}") (
      builtins.sort builtins.lessThan (builtins.attrNames (builtins.readDir dir))
    );
in
{
  pm = final.callPackage ./pm.nix { };
  pm-plugins = final.callPackage ./pm-plugins.nix { };
  derisk = final.callPackage ./derisk.nix { };
  losos-installer = final.callPackage ./losos-installer.nix { };
  losos-security = final.callPackage ./losos-security.nix { };
  losos-swap = final.callPackage ./losos-swap.nix { };
  # Halium (nixos/modules/halium.nix): Android's HAL headers, and libhybris,
  # which loads the vendor's bionic-linked GPU and HAL libraries into glibc
  # processes.
  android-headers = final.callPackage ./android-headers.nix { };
  libhybris = final.callPackage ./libhybris.nix { };
  # The Android Translation Layer (nixos/modules/atl.nix), which nixpkgs does
  # not carry either; only in the closure when losos.android.enable is set.
  android-translation-layer = final.callPackage ./android-translation-layer.nix { };
  # Uranium (uranium.nix), the web browser. The default build wraps
  # nixpkgs' ungoogled-chromium as nixpkgs alone builds it: taken from this
  # package set, it would link the patched GTK below and need compiling,
  # which takes longer than CI's time budget, while nixpkgs' own build
  # substitutes from cache.nixos.org. ungoogled-chromium, not chromium, so
  # the browser has no Google API keys, no Safe Browsing lookups, no
  # field trials and none of Google's domains to reach (docs/nixos.md,
  # "Uranium"). uranium-patched compiles it with patches/chromium;
  # losos.uranium.patched picks it for the image.
  uranium =
    let
      plain = import final.path { inherit (final.stdenv.hostPlatform) system; };
    in
    final.callPackage ./uranium.nix {
      chromium-unwrapped = plain.ungoogled-chromium.browser;
      # The ungoogled-chromium patch series that build applies, which is
      # not in its passthru: the same call nixpkgs makes, so the same path.
      ungoogler = plain.callPackage (
        final.path + "/pkgs/applications/networking/browsers/chromium/ungoogled.nix"
      ) { } { inherit (plain.ungoogled-chromium.upstream-info.deps.ungoogled-patches) rev hash; };
    };
  uranium-patched = final.uranium.override { patched = true; };
  uranium-tabs = final.callPackage ./uranium-tabs.nix { };
  # Not a package: every package in nixpkgs, as a pm package's contents.
  pm-payloads = import ./pm-payloads.nix { inherit (final) lib pkgsStatic runCommand; };

  # Mobile-friendly toolkits (docs/nixos.md, "GTK and Qt on a phone").
  # Replacing them here, rather than patching each application, gives every
  # package that links them the patched build, on the PC and Halium alike.
  # Each patch switches on only when every screen is phone-sized (under 600
  # logical pixels on its short side, derisk's own phone line), so a PC sees
  # stock behaviour; Qt's touch scrolling alone follows a touchscreen instead. The cost is that GTK, Qt and everything built against
  # them no longer substitute from cache.nixos.org and are built by CI.
  #
  # GTK3: Purism's adaptive series, which PureOS and Mobian ship on phones:
  # an adaptive file chooser, about, print and shortcuts windows built from
  # libhandy widgets copied into GTK, maximized dialogs, a back button in
  # dialog header bars, and touch event fixes. 0033 is ours and turns it on
  # from the screen size instead of a per-device setting.
  gtk3 = prev.gtk3.overrideAttrs (old: {
    patches = (old.patches or [ ]) ++ patchesIn ./patches/gtk3;
  });
  # GTK4: Purism's three adaptive patches (postmarketOS carried the same two
  # behaviour changes until libadwaita 1.5 made its own dialogs adaptive):
  # resizable dialogs and transient windows open maximized, and get only a
  # close button. Plain GTK4 windows still need them; libadwaita's
  # AdwDialog and breakpoints already adapt and are left alone. 0004 is ours,
  # as in GTK3.
  gtk4 = prev.gtk4.overrideAttrs (old: {
    patches = (old.patches or [ ]) ++ patchesIn ./patches/gtk4;
  });
  # Qt 6: nobody ships Qt phone patches (Plasma Mobile adapts in Kirigami
  # and its own shell), so both are ours. A finger scrolls Qt Widgets'
  # scroll areas kinetically, and resizable dialogs open maximized with
  # their minimum size capped at the screen. Since Qt 6.10 the Wayland
  # client lives in qtbase, so qtwayland, now only the compositor library,
  # needs nothing. Qt 5 is left stock: nothing in the image links it.
  qt6 = prev.qt6.overrideScope (
    _: qtPrev: {
      qtbase = qtPrev.qtbase.overrideAttrs (old: {
        patches = (old.patches or [ ]) ++ patchesIn ./patches/qtbase;
      });
    }
  );
}
