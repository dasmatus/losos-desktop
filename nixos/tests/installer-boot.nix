# Boot the installer from its actual ISO under UEFI, including the initrd
# mounts and the service that owns tty1.
{ self }:

let
  # The test driver talks to a guest only through the backdoor shell that
  # nixpkgs' test-instrumentation.nix starts on hvc0. A test node gets that
  # module for free, but the ISO is a system of its own, evaluated in
  # installer.nix, so without it every machine.succeed waits for a shell that
  # never answers until the driver's global timeout kills the run. The module
  # is added to that system here, not to the medium that ships.
  installerIso =
    (self.nixosConfigurations."losos-desktop-x86_64".config.system.build.installerSystem.extendModules {
      modules = [
        (
          { modulesPath, ... }:
          {
            imports = [ "${modulesPath}/testing/test-instrumentation.nix" ];
          }
        )
      ];
    }).config.system.build.isoImage;
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
    machine.wait_for_unit("derisk-installer.service")
    machine.succeed(
        "systemctl show derisk-installer.service --property=TTYPath --value "
        "| grep -qx /dev/tty1"
    )
  '';
}
