# How it boots

Every stage is systemd. That is the point of the design, not a side effect of
it: there is no shim script, no initramfs generator from another project, no
partitioning tool, no update agent, and no session supervisor that isn't PID 1
or a user instance of it.

```
firmware
  -> systemd-boot            picks an entry from the ESP
  -> losos.efi               the UKI: systemd-stub + kernel + initrd + cmdline
  -> systemd-stub            measures its sections into PCR 11, execs the kernel
  -> kernel                  unpacks the initrd, execs /init
  -> systemd (initrd)        -Dinitrd=true; the same binary as on the real root
       systemd-repart            creates/grows ESP, root, /home on first boot
       systemd-veritysetup       verifies /usr
       systemd-cryptsetup        unlocks, if enrolled
       systemd-growfs            grows the filesystem to the partition
  -> switch-root
  -> systemd (system)
       systemd-firstboot         locale, timezone, hostname
       systemd-sysusers          writes /etc/passwd from sysusers.d
       systemd-tmpfiles          creates the directories a fresh root lacks
       systemd-userdbd/-homed    users exist here, not in /etc/passwd
       systemd-networkd/-resolved/-timesyncd
       systemd-bless-boot        marks this boot good
  -> gdm.service
  -> systemd-logind             seat, session, device fds
  -> user@.service              the user manager
       graphical-session.target     the whole GNOME session, as user units
```

## The UKI

`recipes/90-image/losos-image/files/mkuki.py` appends six PE sections to
systemd's `linuxx64.efi.stub`:

| Section | Contents |
|---|---|
| `.linux` | the kernel |
| `.initrd` | the xz-compressed newc initramfs |
| `.cmdline` | `files/cmdline` |
| `.osrel` | `/usr/lib/os-release`, so the stub can identify itself before any filesystem is mounted |
| `.uname` | the kernel version |
| `.sbat` | the revocation record shim uses |

One file, so one thing to sign and one thing to measure. The kernel, its
initrd and its command line cannot drift apart, which is what makes a
TPM2-bound secret meaningful: `systemd-stub` extends PCR 11 with each section,
so a modified command line changes the measurement and the secret does not
unseal.

Signing is not done here. That needs a key, and a key belongs to whoever owns
the machine, not to a build recipe.

## First boot creates the disk

There is no disk image. `systemd-repart` runs in the initrd and creates what
`overlay/usr/lib/repart.d/` describes:

- `10-esp.conf` — 512 MB ESP, sized for several UKIs side by side so an A/B
  update needs no repartitioning;
- `20-root.conf` — root, `Type=root-x86-64`, grown to the disk;
- `30-home.conf` — `/home`, weighted to take the remainder.

`systemd-gpt-auto-generator` then finds the root filesystem by GPT partition
type UUID. That is why there is no `/etc/fstab` in this OS and no `root=` on the
kernel command line: both would be restating something the partition table
already says.

## Users do not exist until someone makes one

`/etc` ships effectively empty. `systemd-sysusers` writes `/etc/passwd` on first
boot from `overlay/usr/lib/sysusers.d/losos.conf`, and that file contains only
system users — `gdm`, `polkitd`, `systemd-oom`.

Human users are `systemd-homed` records: a signed identity plus a LUKS image on
the `/home` partition, created with `homectl`. gdm sees them through ordinary
NSS calls, answered by `libnss_systemd` via `systemd-userdbd`. There is no
`useradd` in the image.

The reason is the update model below: an A/B replacement of the root partition
throws away everything in `/etc`, and a user account that lived there would go
with it.

## Updates replace, they do not patch

`systemd-sysupdate` keeps two instances of each thing it manages
(`overlay/usr/lib/sysupdate.d/`): the UKI in the ESP, and the root partition.
An update downloads the new one beside the running one and touches neither the
running system nor `/home`.

Rollback needs no failure detection. `systemd-boot` offers the newest entry;
`systemd-bless-boot` marks it good only once
`systemd-boot-check-no-failures` is satisfied; an entry that never gets blessed
runs out of tries and the firmware falls back to the previous one by itself.

The `Path=https://example.invalid/` in those files is deliberate: there is no
update server, and pointing at a real one would be a claim this repository
cannot back.

## The session is systemd all the way down

`gnome-session` is built `-Dsystemd_session=enabled`, so the session is a tree
of systemd **user** units under `graphical-session.target` rather than
gnome-session's own process supervision. `systemd-xdg-autostart-generator`
pulls legacy `.desktop` autostart entries into the same tree.

What that buys, concretely: `systemctl --user status` describes the desktop;
a crashed component is restarted by the same supervisor as everything else;
and `systemd-oomd` can act on a per-application cgroup under memory pressure —
which is what `overlay/usr/lib/systemd/system/user@.service.d/10-oomd.conf`
opts every session into. Without that drop-in oomd watches nothing, and the
kernel OOM killer takes the compositor instead.

`mutter` and `gdm` get their DRM and input file descriptors from
`systemd-logind`, which is what lets the compositor run without root.

## What has not been tested

None of the above has run. See [`limits.md`](limits.md): there is no VM here,
no EFI firmware and no loop device. The UKI is verified structurally and
nothing more.
