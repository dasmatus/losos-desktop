# Users do not exist until someone makes one, and then a user IS a LUKS volume.
#
# There is no human user in users.users and no /etc/passwd entry for one. A
# person is a systemd-homed record: a signed identity plus an encrypted home
# image on the /home partition, created with homectl. greetd and its PAM
# stack see them through NSS, answered by nss-systemd from systemd-userdbd.
# The reason is the update model: /etc can be reset (disk.nix) and a user who
# lived there would go with it, while a homed user travels with their own
# home area.
#
# This is also where NixOS closes a gap the pm tree never did. That tree
# shipped no PAM configuration at all -- Linux-PAM's tarball carries none and
# gdm installed none -- so docs/limits.md had to say that nothing in the image
# could authenticate anyone. NixOS generates the PAM stack, and with homed
# enabled it puts pam_systemd_home into all four management groups of every
# service, which is the condition for a login to actually open the home area
# rather than succeed and find it locked.
{ config, lib, ... }:

{
  services.homed = {
    enable = true;

    # systemd-homed-firstboot: on the first boot, before any login screen is
    # reachable, ask for the first user on the console. Without it a fresh
    # install reaches the login screen with nobody to log in as, because there is no
    # useradd here and no account in the image.
    promptOnFirstBoot = true;

    settings.Home = {
      # LUKS, stated rather than inherited: it is the whole design that the
      # user's volume is the encrypted thing, while root is not.
      DefaultStorage = "luks";
      DefaultFileSystemType = "ext4";
    };
  };

  # Upstream's wizard skips the group prompt. The first user is the only one
  # who can then create others or ask for a factory reset, and both are gated
  # on wheel, so this one is asked.
  systemd.services.systemd-homed-firstboot.serviceConfig.ExecStart = [
    ""
    "${config.systemd.package}/bin/homectl firstboot --prompt-new-user --prompt-shell=no --prompt-groups=yes --mute-console=yes"
  ];

  # userdbd serves homed's records over Varlink to NSS and to everything else
  # that asks. The homed module enables it; it is named here because it is
  # half of how a user exists at all.
  # userdbd serves homed's records over Varlink to NSS and to everything else
  # that asks. The homed module enables it; it is named here because it is
  # half of how a user exists at all.
  #
  # It used to set silenceHighSystemUsers, for gdm's greeter users at 60578
  # and up. greetd's one `greeter` user takes a UID from the ordinary system
  # range, so nothing is left to silence, and the warning stays on to catch a
  # system user that could make homed's first-boot wizard think a regular
  # user already exists. The VM test's userdbctl step checks the same thing.
  services.userdbd.enable = true;

  # NixOS routes NSS through a caching daemon so that glibc can find
  # nss-systemd in the store, and the homed module asserts it. This is the one
  # place a non-systemd daemon is load-bearing for accounts, and it is nsncd:
  # a stateless forwarder, not a cache with its own idea of who exists.
  services.nscd.enable = true;

  # No root password and no root login. Administration is run0, which asks
  # polkit, which asks a wheel member to authenticate -- as that member.
  #
  # NixOS refuses a configuration in which neither root nor any wheel user in
  # users.users can log in, to stop people locking themselves out. Every
  # administrator here is a homed user the NixOS module system never sees, so
  # that check cannot be satisfied and is turned off rather than defeated with
  # a password nobody should have.
  users.allowNoPasswordLogin = true;

  # run0 instead of sudo: a transient unit started by the service manager
  # rather than a setuid binary inheriting the caller's environment.
  security.sudo.enable = false;
  security.run0.enableSudoAlias = true;
}
