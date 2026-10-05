# Flatpak, with Flathub as the remote and Bazaar as the store.
#
# Apps the image does not ship come from Flathub as Flatpaks: sandboxed,
# updated on their own schedule, and kept in /var/lib/flatpak on the root
# partition, so a sysupdate of /usr leaves them alone and a factory reset
# removes them with the rest of the state.
{
  config,
  pkgs,
  ...
}:

let
  flatpak = config.services.flatpak.package;
in
{
  # Installs flatpak's D-Bus system helper, its polkit rules, the `flatpak`
  # system user and the /var/lib/flatpak/exports entries on XDG_DATA_DIRS. It
  # needs xdg.portal, which desktop.nix enables with derisk's backend.
  #
  # flatpak's own polkit rule lets an active, local wheel member install,
  # update and remove apps for the whole system without a password. That is
  # Bazaar's normal path: derisk has no polkit agent to ask anyone else, so a
  # user outside wheel installs for themselves (`--user`) instead.
  services.flatpak.enable = true;

  # Flathub, added once at boot and kept by flatpak afterwards. The repo file
  # is in the image rather than fetched, so the signing key every Flathub
  # install is verified against is the one this tree pins and reviews, not
  # whatever an HTTPS fetch at first boot returns. --if-not-exists makes it a
  # no-op on every later boot, and leaves a remote the user changed alone.
  systemd.services.flatpak-flathub = {
    description = "Add the Flathub Flatpak remote";
    wantedBy = [ "multi-user.target" ];
    # The system helper and /var/lib/flatpak's tmpfiles are both in place by
    # then; adding a remote needs neither the network nor the helper.
    after = [ "systemd-tmpfiles-setup.service" ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      ExecStart = "${flatpak}/bin/flatpak remote-add --system --if-not-exists flathub ${./flathub.flatpakrepo}";
    };
  };

  # Bazaar, the store: a GTK 4 front end to the remotes flatpak already has,
  # Flathub first. It keeps its catalogue in a session service
  # (io.github.kolunmi.Bazaar.service, D-Bus-activated) so the window opens
  # on loaded state; systemd.packages and dbus.packages put that user unit
  # and its activation file where the session finds them.
  environment.systemPackages = [ pkgs.bazaar ];
  systemd.packages = [ pkgs.bazaar ];
  services.dbus.packages = [ pkgs.bazaar ];
}
