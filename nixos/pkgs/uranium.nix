# Uranium, the OS's web browser: Chromium under its own name, which on a
# phone introduces itself and lays pages out as Chrome for Android does, so
# sites send their Android version (docs/nixos.md, "Uranium").
#
# Two builds share this launcher. `patched` compiles Chromium with
# patches/chromium and rebrand.py: the name inside the browser, its own
# profile directory, app id and icon, and the Android behaviour behind two
# switches. Unpatched, it wraps nixpkgs' own Chromium as cache.nixos.org has
# it, and the launcher gets as close as switches allow: the name everywhere
# outside the browser's own windows, and the Android User-Agent string, but
# not the client hints or the Android viewport. Chromium takes far longer to
# compile than a CI runner's time budget, so the image ships the unpatched
# one until a builder can produce the other (losos.uranium.patched).
{
  lib,
  runCommand,
  makeDesktopItem,
  runtimeShell,
  # nixpkgs' Chromium build, chromium.browser, without its wrapper.
  chromium-unwrapped,
  patched ? false,
  wayland-utils,
  gawk,
  glib,
  gtk3,
  gtk4,
  gsettings-desktop-schemas,
  adwaita-icon-theme,
  libva,
  pipewire,
  wayland,
  libkrb5,
  xdg-utils,
  zstd,
  python3,
}:

let
  browser =
    if patched then
      chromium-unwrapped.overrideAttrs (old: {
        pname = "uranium-unwrapped";
        patches = (old.patches or [ ]) ++ patchesIn ./patches/chromium;
        # After nixpkgs' own postPatch, which leaves the shell in the source
        # root. The name is in about 700 strings and their translations, so a
        # script renames it rather than a patch that every release would break.
        postPatch = (old.postPatch or "") + ''
          python3 ${./uranium/rebrand.py}
        '';
      })
    else
      chromium-unwrapped;

  patchesIn =
    dir:
    map (name: dir + "/${name}") (
      builtins.sort builtins.lessThan (builtins.attrNames (builtins.readDir dir))
    );

  # Chrome for Android's whole User-Agent string since the reduction, the
  # one the unpatched build can send. The patched one builds the same string
  # itself and adds the client hints to match.
  androidUserAgent = "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/${lib.versions.major browser.version}.0.0.0 Mobile Safari/537.36";

  # What nixpkgs' wrapper puts on the library path: VA-API, screen sharing
  # through PipeWire, Wayland, Kerberos, and GTK, which Chromium opens by
  # name for its dialogs and theme. GTK comes from this package set, so a
  # phone gets the adaptive file chooser.
  libraryPath = lib.makeLibraryPath [
    libva
    pipewire
    wayland
    gtk3
    gtk4
    libkrb5
  ];
  dataDirs = lib.concatStringsSep ":" [
    (glib.getSchemaDataDirPath gsettings-desktop-schemas)
    (glib.getSchemaDataDirPath gtk3)
    (glib.getSchemaDataDirPath gtk4)
    "${adwaita-icon-theme}/share"
  ];

  launcher = ''
    #!${runtimeShell}
    # Uranium's launcher: the environment nixpkgs' Chromium wrapper sets, then
    # the switches that make it Uranium, and on a phone, Android.

    # Chromium falls back to its namespace sandbox when NixOS has no setuid
    # helper installed, as nixpkgs' wrapper arranges.
    if [ -x /run/wrappers/bin/${browser.passthru.sandboxExecutableName} ]; then
      export CHROME_DEVEL_SANDBOX=/run/wrappers/bin/${browser.passthru.sandboxExecutableName}
    else
      export CHROME_DEVEL_SANDBOX=${browser.sandbox}/bin/${browser.passthru.sandboxExecutableName}
    fi
    # Desktop shortcuts Chromium writes for web apps run this launcher, and
    # uranium.desktop names its windows: Chromium takes the Wayland app id
    # from it, which is how derisk finds the launcher entry and the icon.
    export CHROME_WRAPPER=uranium
    export CHROME_DESKTOP=uranium.desktop
    export LD_LIBRARY_PATH="''${LD_LIBRARY_PATH:+$LD_LIBRARY_PATH:}${libraryPath}"
    export XDG_DATA_DIRS="${dataDirs}''${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"
    # xdg-open and friends, last, so the session's own come first.
    export PATH="''${PATH:+$PATH:}${xdg-utils}/bin"

    # A phone is every screen under 600 logical pixels on its short side, the
    # line derisk and the GTK and Qt patches draw. It is decided once, at
    # launch: a phone docked to a monitor gets the desktop browser from the
    # next launch on. URANIUM_FORM_FACTOR=phone or desktop overrides it.
    form_factor() {
      case "''${URANIUM_FORM_FACTOR-}" in
        phone | desktop)
          echo "$URANIUM_FORM_FACTOR"
          return
          ;;
      esac
      if [ -z "''${WAYLAND_DISPLAY-}" ]; then
        echo desktop
        return
      fi
      # xdg-output's logical size is the size after scaling, the one the
      # 600 pixel line is in. No output listed means no answer: desktop.
      ${wayland-utils}/bin/wayland-info -i zxdg_output_manager_v1 2>/dev/null |
        ${gawk}/bin/awk '
          /logical_width:/ {
            gsub(",", "")
            width = $2; height = $4
            outputs++
            if ((width < height ? width : height) >= 600) large = 1
          }
          END { print (outputs > 0 && !large) ? "phone" : "desktop" }'
    }

    # Prepended, so a switch given on the command line, which comes later,
    # wins over the launcher's.
    #
    # Wayland, and text-input-v3 for the input method, the protocol derisk's
    # compositor serves its on-screen keyboard over; Chromium speaks v1
    # unless told. --class names the app id outright as well.
    set -- \
      --ozone-platform-hint=auto \
      --enable-wayland-ime \
      --wayland-text-input-version=3 \
      --class=uranium \
      "$@"
    ${lib.optionalString (!patched) ''
      # The patched build keeps its profile in ~/.config/uranium itself;
      # nixpkgs' would use ~/.config/chromium, shared with any stock Chromium.
      set -- --user-data-dir="''${XDG_CONFIG_HOME:-$HOME/.config}/uranium" "$@"
    ''}
    if [ "$(form_factor)" = phone ]; then
      # The touch layout of the tab strip and toolbar, overlay scrollbars as
      # on Android, and touch events on for every page, which sites test for
      # before they serve their touch version.
      #
      # The rest is Chrome for Android's power saving, which the desktop
      # build has but leaves off: a page hidden past its grace period is
      # frozen and stops running timers (stop-in-background, on by default
      # only on Android), a frozen page gives back its memory
      # (MemoryPurgeOnFreeze, likewise), and every background tab but the
      # last one used can be frozen (InfiniteTabsFreezing), where Android
      # freezes whatever is not on screen. The desktop freezing policy's
      # exemptions still hold, such as a tab that is playing audio.
      set -- \
        --top-chrome-touch-ui=enabled \
        --enable-features=OverlayScrollbar,stop-in-background,MemoryPurgeOnFreeze,InfiniteTabsFreezing:num_protected_tabs/1 \
        --touch-events=enabled \
        "$@"
      ${
        if patched then
          ''
            # The two upstream switches patches/chromium make mean Android on
            # Linux, the User-Agent string and client hints, and the viewport,
            # and the phone interface 0004 adds: no tab strip, a tab switcher.
            set -- --use-mobile-user-agent --enable-viewport --uranium-phone-ui "$@"
          ''
        else
          ''
            # Stock Chromium has no Android mode to turn on: --enable-viewport
            # alone would ignore every page's viewport tag. The string is what
            # it can send; its client hints still say Linux.
            set -- --user-agent=${lib.escapeShellArg androidUserAgent} "$@"
          ''
      }
    fi

    exec -a "$0" ${browser}/libexec/chromium/chromium "$@"
  '';

  # The patched build's patch phase, on the files it touches, from the
  # Chromium source nixpkgs pins: patches/chromium apply, and rebrand.py
  # renames every string and keeps every translation. Compiling is the part
  # CI cannot afford; this is the part that breaks when nixpkgs moves
  # Chromium, and it needs only the main source tarball, not its
  # dependencies. nixpkgs' own patches touch none of these files.
  patchCheck =
    runCommand "uranium-patch-check-${chromium-unwrapped.version}"
      {
        nativeBuildInputs = [
          zstd
          python3
        ];
      }
      ''
        mkdir src && cd src
        tar --zstd -xf ${chromium-unwrapped.passthru.chromiumDeps.src} --wildcards \
          './chrome/app/theme/chromium/BRANDING' \
          './chrome/app/chromium_strings.grd' \
          './chrome/app/settings_chromium_strings.grdp' \
          './chrome/app/resources/chromium_strings_*.xtb' \
          './components/components_chromium_strings.grd' \
          './components/strings/components_chromium_strings_*.xtb' \
          './tools/grit/*' \
          ${lib.concatMapStringsSep " " (file: "'./${file}'") [
            "components/embedder_support/user_agent_utils.cc"
            "content/browser/web_contents/web_contents_impl.cc"
            "chrome/common/chrome_paths_linux.cc"
            "chrome/common/channel_info_posix.cc"
            "chrome/browser/shell_integration_linux.cc"
            "chrome/browser/ui/views/toolbar/toolbar_view.cc"
            "chrome/browser/ui/views/frame/browser_view.cc"
          ]}
        for p in ${lib.escapeShellArgs (patchesIn ./patches/chromium)}; do
          echo "applying $p"
          patch -p1 --forward --no-backup-if-mismatch < "$p"
        done
        python3 ${./uranium/rebrand.py}
        grep -qx 'PRODUCT_FULLNAME=Uranium' chrome/app/theme/chromium/BRANDING
        # The product name GRIT builds into every locale's strings.
        grep -A1 'name="IDS_PRODUCT_NAME" desc="The Chrome application name"' \
          chrome/app/chromium_strings.grd | grep -q Uranium
        touch $out
      '';

  desktopItem = makeDesktopItem {
    name = "uranium";
    desktopName = "Uranium";
    genericName = "Web Browser";
    comment = "Browse the web";
    exec = "uranium %U";
    icon = "uranium";
    startupNotify = true;
    startupWMClass = "uranium";
    categories = [
      "Network"
      "WebBrowser"
    ];
    mimeTypes = [
      "text/html"
      "text/xml"
      "application/xhtml+xml"
      "application/pdf"
      "x-scheme-handler/http"
      "x-scheme-handler/https"
    ];
    actions = {
      new-window = {
        name = "New Window";
        exec = "uranium";
      };
      new-private-window = {
        name = "New Incognito Window";
        exec = "uranium --incognito";
      };
    };
  };
in
runCommand "uranium-${browser.version}"
  {
    inherit (browser) version;
    inherit launcher;
    passAsFile = [ "launcher" ];
    passthru = {
      unwrapped = browser;
      inherit patched patchCheck;
    };
    meta = {
      description = "Chromium as Uranium, which turns into Chrome for Android on a phone";
      inherit (browser.meta) license platforms;
      mainProgram = "uranium";
    };
  }
  ''
    install -Dm755 $launcherPath $out/bin/uranium
    ${runtimeShell} -n $out/bin/uranium
    install -Dm644 ${desktopItem}/share/applications/uranium.desktop \
      $out/share/applications/uranium.desktop
    install -Dm644 ${./uranium/uranium.svg} $out/share/icons/hicolor/scalable/apps/uranium.svg
  ''
