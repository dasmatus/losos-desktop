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
  options.losos.android.enable = lib.mkEnableOption ''
    running Android apps on the desktop through the Android Translation Layer.
    This installs ATL and makes it the default handler for .apk files
  '';

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [
      pkgs.android-translation-layer
      atlDesktopItem
    ];

    # Make ATL the default application for APKs, so the file manager's "Open"
    # routes to it. The MIME type is the one Android itself uses for packages.
    xdg.mime = {
      enable = true;
      defaultApplications."application/vnd.android.package-archive" = "android-translation-layer.desktop";
    };

    # ATL renders with GTK4 + Vulkan and talks to the GPU directly; the desktop
    # already enables hardware.graphics, but keep it explicit so enabling
    # Android support alone is sufficient.
    hardware.graphics.enable = true;
  };
}
