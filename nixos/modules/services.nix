# The rest of what the pm tree's preset enabled, and the one service this
# repository writes itself.
{
  config,
  lib,
  pkgs,
  ...
}:

{
  # journald keeps the journal on the root partition, which is state and
  # survives an update but not a factory reset. Crashes go to
  # systemd-coredump and firmware panic records to systemd-pstore, both of
  # which NixOS enables already.
  services.journald.storage = "persistent";

  # Containers, VMs and portable services: systemd-nspawn, systemd-vmspawn,
  # machined, importd and portabled are all in the systemd package NixOS ships,
  # present but not started until someone asks.
  #
  # System and configuration extensions, merged at boot if any are installed.
  # On this OS /usr holds little but the Nix store, so a sysext can add
  # programs under /usr/bin but not replace anything the store provides.
  systemd.additionalUpstreamSystemUnits = [
    "systemd-sysext.service"
    "systemd-confext.service"
  ];
  systemd.services.systemd-sysext.wantedBy = [ "sysinit.target" ];
  systemd.services.systemd-confext.wantedBy = [ "sysinit.target" ];

  # Where a person drops an extension, on a root partition that first boot
  # created empty. /var/lib/portables and /var/lib/machines used to be listed
  # here too; NixOS installs systemd's own portables.conf and
  # systemd-nspawn.conf, which create both with the same modes.
  systemd.tmpfiles.settings."10-losos" = {
    "/var/lib/extensions".d.mode = "0755";
    "/var/lib/confexts".d.mode = "0755";
  };

  # The operating-system security report, on the system bus, for a settings
  # panel to show, as GNOME's Privacy & Security panel did: TPM, disk
  # encryption, verity, kernel lockdown, module signing, IOMMU, the hardening
  # sysctls.
  #
  # Bus-activated, not enabled. The only thing that asks for it is the panel
  # opening its security dialog, and a report that takes milliseconds does not
  # need a process sitting in memory between the two times a year anyone looks.
  services.dbus.packages = [
    (pkgs.writeTextDir "share/dbus-1/system-services/io.losos.Security1.service" ''
      # SystemdService= rather than Exec= alone: dbus hands the request to
      # systemd, which starts the unit with the sandbox below. Activating the
      # binary directly would start it with the bus daemon's environment and
      # none of that sandbox.
      [D-BUS Service]
      Name=io.losos.Security1
      Exec=${lib.getExe pkgs.losos-security} --serve
      User=root
      SystemdService=losos-security.service
    '')
    (pkgs.writeTextDir "share/dbus-1/system.d/io.losos.Security1.conf" ''
      <?xml version="1.0" encoding="UTF-8"?>
      <!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
       "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
      <!--
        Read access is open to everyone. The report says whether this machine
        has a TPM, an encrypted disk and a locked-down kernel; it contains no
        secret, and no method here changes anything, so there is nothing to
        authorise.
      -->
      <busconfig>
        <policy user="root">
          <allow own="io.losos.Security1"/>
        </policy>
        <policy context="default">
          <allow send_destination="io.losos.Security1" send_interface="io.losos.Security1"/>
          <allow send_destination="io.losos.Security1" send_interface="org.freedesktop.DBus.Properties"/>
          <allow send_destination="io.losos.Security1" send_interface="org.freedesktop.DBus.Introspectable"/>
          <allow send_destination="io.losos.Security1" send_interface="org.freedesktop.DBus.Peer"/>
        </policy>
      </busconfig>
    '')
  ];

  systemd.services.losos-security = {
    description = "Operating system security checks";
    after = [ "dbus.socket" ];
    requires = [ "dbus.socket" ];

    serviceConfig = {
      Type = "dbus";
      BusName = "io.losos.Security1";
      ExecStart = "${lib.getExe pkgs.losos-security} --serve";

      # Root, because two of the paths it reads are not world-readable: the
      # SecureBoot efivar and /sys/kernel/security/lockdown. Everything else a
      # root process could do is taken away. A security reporter that is
      # itself an attack surface subtracts from what it reports on.
      User = "root";
      CapabilityBoundingSet = "";
      AmbientCapabilities = "";
      NoNewPrivileges = true;

      # No writes anywhere, and /proc/sys read-only: the sysctl checks must
      # never be able to change a value to improve their own report.
      ProtectSystem = "strict";
      ProtectHome = true;
      ProtectKernelTunables = true;
      ProtectKernelModules = true;
      ProtectKernelLogs = true;
      ProtectControlGroups = true;
      ProtectClock = true;
      ProtectHostname = true;
      ProtectProc = "invisible";
      PrivateTmp = true;
      PrivateDevices = true;

      # The system bus is a unix socket, so nothing here needs IP.
      PrivateNetwork = true;
      RestrictAddressFamilies = "AF_UNIX";

      RestrictNamespaces = true;
      RestrictRealtime = true;
      RestrictSUIDSGID = true;
      LockPersonality = true;
      MemoryDenyWriteExecute = true;
      SystemCallArchitectures = "native";
      SystemCallFilter = "@system-service";
      SystemCallErrorNumber = "EPERM";

      # A ceiling, not an idle timeout -- systemd has none for a bus service.
      # Every request re-reads the files, so being stopped costs the next
      # caller a process start and nothing else.
      RuntimeMaxSec = 300;
    };
  };
}
