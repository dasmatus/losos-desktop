# pm stays the system manager. This image comes from nixpkgs, but software a
# user builds or runs on the machine goes through pm: signed recipes built into
# .cpkg archives, their entrypoints run in pm's sandbox, and pmd as its worker.
#
# Nothing extra is needed for that sandbox. It is unprivileged user namespaces
# plus landlock, both of which NixOS's kernel allows by default.
{ pkgs, ... }:

{
  environment.systemPackages = [ pkgs.pm ];
}
