# libvirt, with QEMU and KVM, so virtual machines are the system's to start,
# list and stop: virt-manager's and virsh's, and the ones pm boots for a
# package that ships its own kernel (docs/pm.md, "Virtual machines").
#
# All of it is state under /var/lib/libvirt and /run/libvirt on the root
# partition, so the verity /usr and the update model are untouched, and nothing
# in it is per machine: a PC without hardware virtualisation gets the same
# image, and libvirt reports that its QEMU emulates rather than runs on KVM.
{ lib, pkgs, ... }:

{
  virtualisation.libvirtd = {
    enable = true;
    # QEMU for the host's own architecture. The default, pkgs.qemu, emulates
    # every target QEMU has, which is most of its size and nothing a desktop's
    # machines run.
    qemu.package = pkgs.qemu_kvm;
  };

  # NixOS starts libvirtd at boot as well as on its socket. Socket activation
  # alone is enough here: the daemon starts on the first connection and, with
  # the module's --timeout 120, exits again once no machine runs, so a desktop
  # that never opens a VM never keeps it. libvirt-guests, which resumes the
  # machines that were running at shutdown, still starts it at boot when there
  # are any.
  #
  # This is libvirt's monolithic daemon, not its modular virtqemud and friends:
  # the NixOS module wires only libvirtd (its config, the qemu.conf it copies to
  # /var/lib/libvirt, the stable emulator links), and taking the modular daemons
  # would mean re-doing that by hand for the same API on the same socket path.
  systemd.services.libvirtd.wantedBy = lib.mkForce [ ];

  # Who may use qemu:///system. NixOS grants it to the libvirtd group, but people
  # here are systemd-homed records the module system never sees (accounts.nix),
  # so nobody is in that group. Administrators get it instead, the way Ubuntu and
  # Fedora put their installer's admin user in libvirt's group: managing system
  # machines can hand a guest any host disk, so it is an administrator's power,
  # and it is not asked for again on every connection because virt-manager and
  # pm connect many times a session. Everyone else uses qemu:///session, which
  # runs their machines as themselves and needs no grant.
  security.polkit.extraConfig = ''
    polkit.addRule(function (action, subject) {
      if (action.id === "org.libvirt.unix.manage" && subject.isInGroup("wheel")) {
        return polkit.Result.YES;
      }
      return polkit.Result.NOT_HANDLED;
    });
  '';
}
