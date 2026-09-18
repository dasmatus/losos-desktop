# Factory reset

A factory reset on a package-managed distribution is not a coherent thing to
ask for: there is no record of which files came from a package, which the user
wrote, and which some install script produced on the side. On an image-based
system there is, and systemd implements it — so this OS has a reset button and
does not have a script that walks the filesystem deleting things.

## What actually happens

Nothing is deleted while the session is running. The whole mechanism is three
pieces:

1. **Asking.** `factory-reset.target` is started. systemd sets the
   `FactoryResetRequest` EFI variable and reboots. Since systemd v258 there is
   also a Varlink interface at `/run/systemd/io.systemd.FactoryReset` —
   socket-activated, enabled in `overlay/usr/lib/systemd/system-preset/10-losos.preset`.
   Without one of those two the only way to ask is `systemd.factory_reset=yes`
   on the kernel command line, which a desktop user has no way to reach.

2. **Noticing.** On the next boot `systemd-factory-reset-generator` reads that
   variable and puts the system in factory-reset mode.

3. **Doing.** `systemd-repart` removes every partition whose `repart.d` file
   says `FactoryReset=yes`, then creates it again from the same file that
   created it originally. The reset is *the absence of a partition*, performed
   by the code path that made it.

That is why `overlay/usr/lib/repart.d/30-home.conf` carries `FactoryReset=yes`
and why nothing else in this repository implements a reset.

## What is reset, and what is not

`30-home.conf` — the home partition — is the only one marked. Root is
deliberately not:

- `/usr` lives on the root partition. Removing it would take the operating
  system with it, and repart would then be recreating an empty partition with
  nothing to boot.
- `/etc` and `/var` also live there, so **machine state survives a factory
  reset**: hostname, machine ID, journal, network configuration, systemd-homed
  records outside the home partition.

This is the conservative half of a reset, and calling it a factory reset
overstates it. What would be needed to do better is to move `/var` onto its own
partition and mark that one too — at which point the reset covers everything
except the image itself, which is the correct shape. That is a partitioning
change with an A/B-update interaction (`sysupdate.d` matches on partition
types), so it is written down here rather than done halfway.

## The button

`recipes/30-gnome/gnome-control-center/files/patches/0001-system-panel-add-a-factory-reset-row.patch`
adds a row to the System panel. It starts `factory-reset.target` over the
system bus behind an `AdwAlertDialog`, and the dialog says "the next time this
device starts" because that is when anything happens.

polkit mediates the start through `org.freedesktop.systemd1.manage-units`,
which covers *every* unit — granting it wholesale to provide a reset button
would hand the same session every other service on the machine. So
`overlay/usr/share/polkit-1/rules.d/50-losos-factory-reset.rules` narrows it to
that one unit, that one verb, and a session that is both active and local, and
still returns `AUTH_ADMIN` rather than `YES`: it decides who may be asked, not
who may skip being asked.
