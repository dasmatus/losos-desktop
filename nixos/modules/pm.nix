# pm stays the system manager. This image comes from nixpkgs, but software a
# user builds or runs on the machine goes through pm: signed recipes built into
# .cpkg archives, their entrypoints run in pm's sandbox, and pmd as its worker.
#
# Nothing extra is needed for that sandbox. It is unprivileged user namespaces
# plus landlock, both of which NixOS's kernel allows by default.
{ pkgs, ... }:

#
# pm's plugins are shipped under /run/current-system/sw/share/pm/plugins and
# not installed into anyone's config: pm runs a plugin only when it is signed
# by a key its user trusts, and that trust is the user's to give.
{
  environment.systemPackages = [
    pkgs.pm
    pkgs.pm-plugins
  ];
  environment.pathsToLink = [ "/share/pm" ];
}
