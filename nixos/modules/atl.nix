# Android app support, through the Android Translation Layer.
#
# ATL runs Android apps as native Linux processes: it translates Android's Java
# and NDK APIs onto GTK4, Vulkan and the host's own libraries, so an app shows
# up as an ordinary window in the derisk session rather than inside an emulated
# Android. Off by default: it pulls in a large dependency closure (WebKitGTK,
# FFmpeg, the Vulkan and OpenXR loaders) that an image with no interest in
# Android apps should not carry, and its package is still being brought up
# (see nixos/pkgs/android-translation-layer.nix).
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.losos.android;

  # Open an .apk from the file manager by handing it to ATL. ATL picks the
  # launchable activity itself, so the handler passes only the file. %f, not
  # %F: ATL runs one app per process.
  atlDesktopItem = pkgs.makeDesktopItem {
    name = "android-translation-layer";
    desktopName = "Android app";
    exec = "${lib.getExe pkgs.android-translation-layer} %f";
    mimeTypes = [ "application/vnd.android.package-archive" ];
    # An installed app is launched by ATL, not shown in the app grid itself;
    # this entry exists to be the default handler for APK files.
    noDisplay = true;
  };
in
{
  options.losos.android = {
    enable = lib.mkEnableOption ''
      running Android apps on the desktop through the Android Translation
      Layer. This installs ATL; whether opening an .apk runs it is a separate
      choice, openApks
    '';

    openApks = lib.mkEnableOption ''
      making ATL the default handler for .apk files, so "Open" on one in the
      file manager runs it. Off by default on purpose: ATL runs an app's dex
      and native code as an ordinary process of the session user, with none
      of Android's per-app sandbox, so a malicious APK has the same reach as
      any native program the user runs. Leaving this off keeps "open a
      downloaded file" from meaning "run untrusted code"; an APK is then run
      only by someone who chose to launch ATL on it. Turn it on knowingly, or
      wait for the sandboxed launcher (bubblewrap or a confined transient
      unit) tracked in docs/nixos.md
    '';
  };

  config = lib.mkIf cfg.enable {
    # The handler entry is installed only when openApks is on: without it, an
    # .apk has no default application and nothing runs on "Open".
    environment.systemPackages = [
      pkgs.android-translation-layer
    ]
    ++ lib.optional cfg.openApks atlDesktopItem;

    # Make ATL the default application for APKs, so the file manager's "Open"
    # routes to it. The MIME type is the one Android itself uses for packages.
    # Gated on openApks for the reason its option text gives.
    xdg.mime = lib.mkIf cfg.openApks {
      enable = true;
      defaultApplications."application/vnd.android.package-archive" = "android-translation-layer.desktop";
    };

    # ATL renders with GTK4 + Vulkan and talks to the GPU directly; the desktop
    # already enables hardware.graphics, but keep it explicit so enabling
    # Android support alone is sufficient.
    hardware.graphics.enable = true;
  };
}
