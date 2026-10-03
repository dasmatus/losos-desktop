# Boot the installer from its actual ISO under UEFI, including the initrd
# mounts and the service that owns tty1.
{ self }:

let
  installerIso = self.nixosConfigurations."losos-desktop-x86_64".config.system.build.installerIso;
in
{
  name = "losos-desktop-installer-boot";

  nodes.machine =
    { ... }:
    {
      virtualisation = {
        directBoot.enable = false;
        mountHostNixStore = false;
        useEFIBoot = true;
        qemu.options = [
          "-cdrom ${installerIso}"
          "-boot order=d"
        ];
        memorySize = 3072;
      };
    };

  testScript = ''
    machine.succeed("findmnt --kernel /iso")
    machine.succeed("findmnt --kernel /nix/store")
    machine.wait_for_unit("losos-installer.service")
    machine.succeed(
        "systemctl show losos-installer.service --property=TTYPath --value "
        "| grep -qx /dev/tty1"
    )
  '';
}
