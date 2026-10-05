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
  stdenv,
  runCommand,
  writeShellScript,
  makeDesktopItem,
  ccache,
  runtimeShell,
  # nixpkgs' ungoogled-chromium build, without its wrapper, and the
  # ungoogled-chromium patch series it applies.
  chromium-unwrapped,
  ungoogler,
  patched ? false,
  wayland-utils,
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
  patchutils,
  fetchzip,
  jq,
  uranium-tabs,
  x2mcsapi,
  # The Flatpak's two helpers, linked statically (flatpakPayload).
  uranium-tabs-static,
  wayland-utils-static,
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
        # nixpkgs' gn arguments are fixed in its configurePhase, so these go
        # into args.gn after it and gn runs again over the same directory.
        #
        # An official build already turns on ThinLTO on Linux, and CFI on
        # x86_64: forward-edge checks on virtual calls, casts and indirect
        # calls, so a corrupted function pointer or vtable stops the process
        # instead of jumping where an attacker points it. They are set here
        # so that gn refusing either fails the build rather than quietly
        # building without, and checked after. Chromium does not build CFI
        # for arm64 Linux; there the official build's branch protection
        # (PAC and BTI, arm_control_flow_integrity) does that job in
        # hardware where the CPU has it.
        #
        # cc_wrapper puts the compiler behind ccache when the builder has
        # one (ccacheWrapper), which is what lets CI compile this in rounds.
        postConfigure =
          (old.postConfigure or "")
          + ''
            {
              echo 'use_thin_lto = true'
              echo 'cc_wrapper = "${ccacheWrapper}"'
          ''
          + lib.optionalString stdenv.hostPlatform.isx86_64 ''
            echo 'is_cfi = true'
            echo 'use_cfi_icall = true'
          ''
          + ''
            } >> out/Release/args.gn
            gn gen out/Release
            for arg in use_thin_lto ${lib.optionalString stdenv.hostPlatform.isx86_64 "is_cfi use_cfi_icall"}; do
              if ! gn args out/Release --short --list=$arg | grep -qx "$arg = true"; then
                echo "uranium: gn did not keep $arg = true" >&2
                exit 1
              fi
            done
          '';
      })
    else
      chromium-unwrapped;

  # The compiler, through ccache when the builder offers a cache at
  # /var/cache/uranium-ccache (Nix's extra-sandbox-paths), and directly
  # otherwise. The derivation is the same either way, so a build that used
  # the cache is the one the image asks for, and ccache only returns an
  # object for exactly the same preprocessed input and compiler. Paths are
  # made relative to the build directory, and a source file's time is not
  # held against it, because patching leaves every patched file newer than
  # the cache entry.
  ccacheWrapper = writeShellScript "uranium-cc-wrapper" ''
    if [ -d /var/cache/uranium-ccache ] && [ -w /var/cache/uranium-ccache ]; then
      export CCACHE_DIR=/var/cache/uranium-ccache
      export CCACHE_BASEDIR="$NIX_BUILD_TOP"
      export CCACHE_NOHASHDIR=1
      export CCACHE_SLOPPINESS=include_file_mtime,include_file_ctime,time_macros
      export CCACHE_MAXSIZE=40G
      exec ${lib.getExe ccache} "$@"
    fi
    exec "$@"
  '';

  patchesIn =
    dir:
    map (name: dir + "/${name}") (
      builtins.sort builtins.lessThan (builtins.attrNames (builtins.readDir dir))
    );

  # uBlock Origin Lite, the ad blocker every profile starts with. It is
  # the Manifest V3 uBlock Origin, which blocks through Chromium's own
  # declarativeNetRequest engine with no code reading the pages, and its
  # settings choose among its filter lists: ads, trackers, annoyances,
  # malware domains, and each region's own. The Web Store, which would
  # update it, is out of reach of an ungoogled build, so it updates with
  # the OS.
  #
  # Chromium names an extension loaded from a directory after the
  # directory's path unless its manifest has a key, so a key is added: the
  # id stays mglkdfjpgkabicfgmggmklcngiopcgpk across updates, and with it
  # the user's choice of filter lists. Only the public half exists.
  ublockOriginLite =
    runCommand "ublock-origin-lite-2026.930.1227"
      {
        src = fetchzip {
          pname = "ublock-origin-lite";
          version = "2026.930.1227";
          url = "https://github.com/uBlockOrigin/uBOL-home/releases/download/2026.930.1227/uBOLite_2026.930.1227.chromium.zip";
          stripRoot = false;
          hash = "sha256-RaCPzuREHDZqOGdnd1Ptuqg4oMqA0PNPTVrsZDtlWz4=";
        };
        nativeBuildInputs = [ jq ];
      }
      ''
        cp -r $src $out
        chmod -R u+w $out
        jq --arg key MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA04v0iYZqw8xG8TdCkZVF/yFnasnpoXMCtUFlXwuwP63KpSFA6yCI684AQgpY7ewlSLiBMdrZdFybnS8i9dqKvnXlSfcDpyzA1zqZ58OVTIDA2W1AUKI/B2+zIvXvCLf/oSBP8VE1YROB7er85/IgHcJkpyGzQECnW09/EuhXVHz/XGXm7A50/Aqh3ge/b7YM+j54h7PL7dXso5ffzB8GTGJCtJfqeh+GjTakqfG/TCygm42/fcWTtMBwgDmOOr7olVZKCAbxGyHRF+DSNM1MQK480ABuy+yLjWcmQFiWUHesngOiqauhwG/38cgqfFcostX9M9G6B7khOi3NZYSpzwIDAQAB '. + {key: $key}' $src/manifest.json > $out/manifest.json
      '';

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

  # The launcher, for the store and for the Flatpak, which differ in where
  # things are and in what has to be set up before Chromium runs.
  mkLauncher =
    {
      shell,
      setup,
      waylandInfo,
      ublock,
      ublockName,
      tabs,
      appId,
      exec,
      patched,
      x2mcsapi ? null,
    }:
    ''
      #!${shell}
      # Uranium's launcher: the environment Chromium needs, then the switches
      # that make it Uranium, and on a phone, Android.
      ${setup}
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
        # 600 pixel line is in, as "logical_width: 392, logical_height: 872".
        # No output listed means no answer: desktop. Read in the shell, not
        # awk, which the Flatpak's runtime does not promise.
        ${waylandInfo} -i zxdg_output_manager_v1 2>/dev/null | {
          outputs=0
          large=0
          while read -r key width _ height _; do
            [ "$key" = logical_width: ] || continue
            width=''${width%,}
            outputs=$((outputs + 1))
            if [ "$width" -ge 600 ] && [ "$height" -ge 600 ]; then
              large=1
            fi
          done
          if [ "$outputs" -gt 0 ] && [ "$large" = 0 ]; then
            echo phone
          else
            echo desktop
          fi
        }
      }

      # Prepended, so a switch given on the command line, which comes later,
      # wins over the launcher's.
      #
      # Wayland, and text-input-v3 for the input method, the protocol derisk's
      # compositor serves its on-screen keyboard over; Chromium speaks v1
      # unless told. --class names the app id outright as well.
      #
      # Then nothing the browser sends on its own: no <a ping> requests to the
      # addresses a page lists when a link is followed (--no-pings), and none
      # of the requests Chromium makes with no page asking, such as component
      # and extension update checks and the field trial configuration
      # (--disable-background-networking). The policies below switch off
      # the rest by name.
      #
      # And two extensions, loaded on every launch, so they are in every
      # profile and their version is the OS's: uBlock Origin Lite, and the
      # one that lists the tabs in derisk's command palette
      # (src/uranium-tabs says how). Chromium writes the indexed form of
      # uBlock's filter lists into the extension's own directory, and fails
      # to load it from a read-only one such as the store, so it runs from
      # a copy in the user's data directory, made again when the OS brings
      # a new one.
      ublock_dir="''${XDG_DATA_HOME:-$HOME/.local/share}/uranium/extensions"
      ublock="$ublock_dir/${ublockName}"
      if [ ! -f "$ublock/manifest.json" ]; then
        rm -rf "$ublock_dir"/ublock-origin-lite*
        mkdir -p "$ublock_dir"
        cp -r ${ublock} "$ublock.new"
        chmod -R u+w "$ublock.new"
        mv "$ublock.new" "$ublock"
      fi
      ${lib.optionalString (x2mcsapi != null) ''
        # The interface in mcsapi's look, like every app derisk shows:
        # x2mcsapi writes a GTK theme from the mcsapi theme derisk has
        # published, and Chromium draws its toolbar, tabs, menus and dialogs
        # from the GTK theme (config.nix starts profiles on it). The theme
        # is read at launch; one changed later shows from the next launch.
        # Outside a derisk session x2mcsapi's default, derisk-dark, applies.
        mcsapi_theme=derisk-dark
        if [ -r "''${XDG_RUNTIME_DIR:-/nonexistent}/derisk/theme.json" ]; then
          # theme.json starts {"id":"<id>",; read in the shell.
          read -r line <"$XDG_RUNTIME_DIR/derisk/theme.json" || true
          case "$line" in
            '{"id":"'*)
              line=''${line#'{"id":"'}
              mcsapi_theme=''${line%%'"'*}
              ;;
          esac
        fi
        if ${x2mcsapi} --theme "$mcsapi_theme" install >/dev/null; then
          export GTK_THEME=x2mcsapi
        fi
      ''}
      set -- \
        --ozone-platform-hint=auto \
        --enable-wayland-ime \
        --wayland-text-input-version=3 \
        --class=${appId} \
        --no-pings \
        --disable-background-networking \
        --load-extension="$ublock",${tabs} \
        "$@"
      ${lib.optionalString patched ''
        # One page per window, with no tab strip: the tabs are in derisk's
        # command palette, which uranium-tabs fills (patches/chromium 0004).
        set -- --uranium-single-view "$@"
      ''}
      ${lib.optionalString (!patched) ''
        # The patched build keeps its profile in ~/.config/uranium itself;
        # an unpatched one would use ~/.config/chromium, shared with any stock
        # Chromium.
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

      ${exec} "$@"
    '';

  launcher = mkLauncher {
    shell = runtimeShell;
    setup = ''
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

    '';
    waylandInfo = "${wayland-utils}/bin/wayland-info";
    ublock = ublockOriginLite;
    ublockName = ublockOriginLite.name;
    tabs = ./uranium/tabs;
    appId = "uranium";
    exec = "exec -a \"$0\" ${browser}/libexec/chromium/chromium";
    inherit patched;
    x2mcsapi = lib.getExe x2mcsapi;
  };

  # The files patches/chromium and rebrand.py touch, as the patched build
  # sees them. Its patch phase applies nixpkgs' patches and ours; then
  # nixpkgs' postPatch applies ungoogled-chromium's series, and rebrand.py
  # runs last. Every ungoogled patch is applied here only for its hunks in
  # these files, in the series' order, so a patch of ours that one of
  # theirs no longer applies over fails here and not hours into a build.
  # Compiling is the part CI cannot afford; this is the part that breaks
  # when nixpkgs moves Chromium, and it needs only the main source
  # tarball, not its dependencies. nixpkgs' own patches touch none of
  # these files.
  sourceFiles = [
    "components/embedder_support/user_agent_utils.cc"
    "content/browser/web_contents/web_contents_impl.cc"
    "chrome/common/chrome_paths_linux.cc"
    "chrome/common/channel_info_posix.cc"
    "chrome/browser/shell_integration_linux.cc"
    "chrome/browser/ui/views/toolbar/toolbar_view.cc"
    "chrome/browser/ui/views/frame/browser_view.cc"
    "chrome/browser/ui/views/frame/layout/browser_view_tabbed_layout_impl.cc"
    "chrome/browser/ui/browser_ui_prefs.cc"
    "chrome/browser/extensions/api/settings_private/prefs_util.cc"
    "chrome/browser/resources/settings/appearance_page/appearance_page.html.ts"
  ];
  patchCheck =
    runCommand "uranium-patch-check-${chromium-unwrapped.version}"
      {
        nativeBuildInputs = [
          zstd
          python3
          patchutils
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
          ${lib.concatMapStringsSep " " (file: "'./${file}'") sourceFiles}
        for p in ${lib.escapeShellArgs (patchesIn ./patches/chromium)}; do
          echo "applying $p"
          patch -p1 --forward --no-backup-if-mismatch < "$p"
        done
        grep -v '^#' ${ungoogler}/patches/series | while read -r p; do
          [ -n "$p" ] || continue
          filterdiff -p1 ${
            lib.concatMapStringsSep " " (file: "-i '${file}'") (
              sourceFiles ++ [ "chrome/app/settings_chromium_strings.grdp" ]
            )
          } ${ungoogler}/patches/"$p" > hunks
          if [ -s hunks ]; then
            echo "applying ungoogled-chromium's $p"
            patch -p1 --forward --no-backup-if-mismatch < hunks
          fi
        done
        python3 ${./uranium/rebrand.py}
        grep -qx 'PRODUCT_FULLNAME=Uranium' chrome/app/theme/chromium/BRANDING
        # The product name GRIT builds into every locale's strings.
        grep -A1 'name="IDS_PRODUCT_NAME" desc="The Chrome application name"' \
          chrome/app/chromium_strings.grd | grep -q Uranium
        touch $out
      '';

  # The launcher entry, named after the app id: uranium in the image,
  # org.losos.Uranium in the Flatpak, as Flatpak requires.
  mkDesktopItem =
    id:
    makeDesktopItem {
      name = id;
      desktopName = "Uranium";
      genericName = "Web Browser";
      comment = "Browse the web";
      exec = "uranium %U";
      icon = id;
      startupNotify = true;
      startupWMClass = id;
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
  desktopItem = mkDesktopItem "uranium";

  # Everything of the Flatpak's that is not Chromium itself, laid out as it
  # goes under /app. The Flatpak runs upstream ungoogled-chromium's portable
  # build of the same version (nixos/pkgs/uranium/flatpak), because a
  # Chromium from the store finds its libraries in /nix/store, which no
  # Flatpak has; the rest is built here, the two helper programs statically
  # so they need nothing from the runtime. Chromium in the Flatpak looks in
  # /app/chromium for what it looks for in /etc/chromium in the image (the
  # manifest rewrites the path in the binary), so it gets the same
  # policies, preferences and tabs host.
  flatpakPayload =
    let
      appId = "org.losos.Uranium";
      config = import ./uranium/config.nix { tabsHost = "/app/bin/uranium-tabs"; };
      flatpakLauncher = mkLauncher {
        shell = "/bin/sh";
        setup = ''
          # Chromium sandboxes itself in a Flatpak through cobalt, which the
          # Chromium base app carries, since a Flatpak can neither run a
          # setuid helper nor make user namespaces of its own: cobalt starts
          # the browser cobalt.ini names, with these arguments, and its
          # sandboxed processes through Flatpak's portal.
          export CHROME_WRAPPER=/app/bin/uranium
          export CHROME_DESKTOP=${appId}.desktop
        '';
        waylandInfo = "/app/bin/wayland-info";
        ublock = "/app/share/uranium/ublock-origin-lite";
        ublockName = ublockOriginLite.name;
        tabs = "/app/share/uranium/tabs";
        inherit appId;
        exec = "exec cobalt";
        patched = false;
      };
    in
    runCommand "uranium-flatpak-payload-${browser.version}"
      {
        launcher = flatpakLauncher;
        passAsFile = [ "launcher" ];
        passthru = { inherit appId; };
      }
      ''
        install -Dm755 $launcherPath $out/bin/uranium
        ${runtimeShell} -n $out/bin/uranium
        install -Dm644 ${./uranium/flatpak/cobalt.ini} $out/etc/cobalt.ini
        install -Dm755 ${lib.getExe uranium-tabs-static} $out/bin/uranium-tabs
        install -Dm755 ${wayland-utils-static}/bin/wayland-info $out/bin/wayland-info
        mkdir -p $out/share/uranium
        cp -r ${ublockOriginLite} $out/share/uranium/ublock-origin-lite
        cp -r ${./uranium/tabs} $out/share/uranium/tabs
        ${lib.concatStrings (
          lib.mapAttrsToList (name: value: ''
            install -Dm644 ${builtins.toFile "uranium-config" (builtins.toJSON value)} $out/chromium/${name}
          '') config
        )}
        install -Dm644 ${mkDesktopItem appId}/share/applications/${appId}.desktop \
          $out/share/applications/${appId}.desktop
        install -Dm644 ${./uranium/uranium.svg} $out/share/icons/hicolor/scalable/apps/${appId}.svg
        install -Dm644 ${./uranium/flatpak/org.losos.Uranium.metainfo.xml} \
          $out/share/metainfo/${appId}.metainfo.xml
        chmod -R u+w $out/share/uranium
      '';
in
runCommand "uranium-${browser.version}"
  {
    inherit (browser) version;
    inherit launcher;
    passAsFile = [ "launcher" ];
    passthru = {
      unwrapped = browser;
      inherit patched patchCheck ublockOriginLite;
      chromiumConfig = import ./uranium/config.nix { tabsHost = lib.getExe uranium-tabs; };
      inherit flatpakPayload;
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
