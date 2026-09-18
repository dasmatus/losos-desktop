# The systemd inventory

Every systemd component this OS builds: the meson option that produces it, what
it ships, and where the OS actually uses it.

This document has an executable half. Every component named here appears in
`recipes/10-systemd/systemd/files/inventory.txt`, and a `Test` step in the
systemd recipe fails the build naming any path that is absent. So "as many
systemd components as possible" is a claim that can go red, not a sentence in a
README. If you add a component here, add it there.

The design goal is not a checklist. It is that **every layer of this system is
systemd**: the bootloader, the thing that measures the boot, the thing that
creates the disk, the thing that creates the users, the thing that supervises
the desktop session, and the thing that updates all of it. Where a component is
enabled and then not wired into anything, it says so.

---

## Boot

| Component | Option | Wiring |
|---|---|---|
| `systemd-boot`, `bootctl` | `-Dbootloader=true` | The only bootloader. Installed to the ESP by `systemd-boot-update.service`, which is in the preset. |
| `systemd-stub` (`linuxx64.efi.stub`) | `-Dbootloader=true` | The UKI *is* this stub with `.linux`, `.initrd`, `.cmdline`, `.osrel`, `.uname` and `.sbat` appended. `recipes/90-image/losos-image/files/mkuki.py` does the appending. |
| `ukify` | `-Dukify=enabled` | Shipped for use on the running system; the image build uses `mkuki.py` instead, because `ukify` needs python-pefile and `pip` would grant the whole layer network access (C8). |
| `kernel-install` | `-Dkernel-install=true` | For kernels installed after first boot. |
| `systemd-bless-boot` | `-Dbootloader=true` | Marks a boot good. The other half of A/B rollback: an update that does not reach a working system is not blessed, and `systemd-boot` falls back without anything having to detect the failure. |
| `systemd-boot-check-no-failures` | `-Dbootloader=true` | What "a working system" means above — enabled in the preset. |
| `systemd-bsod` | core | Shows a failure as a full-screen message with a QR code rather than a scrolled-past line. This is a box a person looks at. |

## Measured boot and TPM2

| Component | Option | Wiring |
|---|---|---|
| `systemd-measure` | `-Dtpm2=enabled` | Precomputes the PCR values a UKI will produce. |
| `systemd-pcrextend`, `systemd-pcrphase*` | `-Dtpm2=enabled` | Extends PCR 11 at each boot phase, so a secret can be bound to "the initrd has finished and userspace has not yet started". |
| `systemd-pcrlock` | `-Dtpm2=enabled` | Policy that survives a firmware update, unlike raw PCR binding. |
| `systemd-cryptenroll` | `-Dlibfido2=enabled`, `-Dp11kit=enabled`, `-Dtpm2=enabled` | Enrols a TPM2, a FIDO2 key or a PKCS#11 token as a LUKS factor. Not run automatically: binding a disk to hardware is the owner's decision, and doing it unasked would be a trap. |

## Self-installation

This is why the repository ships no disk image and no installer.

| Component | Option | Wiring |
|---|---|---|
| `systemd-repart` | `-Drepart=true` | Creates the ESP, root and `/home` partitions on first boot from `overlay/usr/lib/repart.d/`, and grows root to the disk it finds itself on. Runs in the initrd. |
| `systemd-gpt-auto-generator` | core | Finds the root filesystem by GPT partition type UUID. This is why there is no `/etc/fstab` and no `root=` on the kernel command line. |
| `systemd-growfs`, `systemd-makefs` | core | The filesystem half of what repart does to a partition. |
| `systemd-firstboot` | `-Dfirstboot=true` | Locale, timezone, hostname on a system whose `/etc` ships empty. |
| `systemd-sysusers` | `-Dsysusers=true` | Writes `/etc/passwd` on first boot from `overlay/usr/lib/sysusers.d/`. There is no `useradd` in the image. |
| `systemd-tmpfiles` | `-Dtmpfiles=true` | Creates the directories a freshly-made root filesystem does not have. |
| `systemd-machine-id-setup` | core | |
| `systemd-creds` | `-Dopenssl=enabled` | Encrypted credentials, TPM-bound, for anything the image should not carry in the clear. |

## Immutability and updates

| Component | Option | Wiring |
|---|---|---|
| `systemd-veritysetup` (+ generator) | `-Dlibcryptsetup=enabled` | dm-verity for `/usr`. |
| `systemd-sysext` | `-Dsysext=true` | Layers extra `/usr` content over an immutable base without modifying it — how you get a compiler onto this machine. In the preset. |
| `systemd-confext` | `-Dsysext=true` | The same for `/etc`. |
| `systemd-sysupdate` (+ `sysupdated`) | `-Dsysupdate=enabled` | A/B updates of both the UKI and the root partition, from `overlay/usr/lib/sysupdate.d/`. Two instances kept, so the previous one is always the rollback target. |

## Identity and accounts

| Component | Option | Wiring |
|---|---|---|
| `systemd-homed`, `homectl` | `-Dhomed=enabled` | **How users exist on this OS.** A user is a signed record plus a LUKS image on the home partition, not a line in `/etc/passwd`. That is what survives an A/B root replacement. |
| `systemd-userdbd`, `userdbctl` | `-Duserdb=true` | The lookup service homed's records are served through. |
| `libnss_systemd` | `-Dnss-systemd=true` | The glue that makes gdm, polkit and everything else see those users through ordinary NSS calls. |
| `pam_systemd`, `pam_systemd_home` | `-Dpam=enabled` | Turns a login into a logind session and unlocks the user's home image at the same moment. |
| `libpwquality` | `-Dpwquality=enabled` | homed refuses a weak password with it. |

## Sessions and the desktop

| Component | Option | Wiring |
|---|---|---|
| `systemd-logind`, `loginctl` | `-Dlogind=true` | Seats, sessions and device access. gdm and mutter get their DRM and input file descriptors from it, which is what lets the compositor run without root. |
| the systemd **user manager** (`user@.service`) | core | Every part of the GNOME session is a user unit under `graphical-session.target`, because `gnome-session` is built `-Dsystemd_session=enabled`. Nothing in the session is supervised by anything other than systemd. |
| `systemd-xdg-autostart-generator` | `-Dxdg-autostart=true` | Turns legacy `.desktop` autostart files into user units, so they are supervised too. |
| `systemd-user-runtime-dir` | core | `/run/user/$UID`. |
| `run0` | core | Replaces `sudo`: no setuid binary, the privileged process is started by PID 1 in a fresh session. |
| `systemd-inhibit` | core | What stops the machine suspending mid-update. |
| `systemd-oomd`, `oomctl` | `-Doomd=true` | Kills the worst-behaved *application* cgroup under memory pressure, while the machine is still responsive. `overlay/usr/lib/systemd/system/user@.service.d/10-oomd.conf` opts every session in — without that drop-in it watches nothing and the kernel OOM killer takes the compositor instead. |

## Network

| Component | Option | Wiring |
|---|---|---|
| `systemd-networkd`, `networkctl` | `-Dnetworkd=true` | The only network stack. `overlay/usr/lib/systemd/network/`. |
| `systemd-resolved`, `resolvectl` | `-Dresolve=true`, `-Ddns-over-tls=openssl` | DNS, DNS-over-TLS opportunistically, mDNS and LLMNR on. |
| `systemd-timesyncd` | `-Dtimesyncd=true` | SNTP. `systemd-time-wait-sync` gates anything that needs a real clock. |
| `systemd-networkd-wait-online` | `-Dnetworkd=true` | `network-online.target`. |
| `systemd-ssh-generator`, `systemd-ssh-proxy` | core (≥256) | Socket-activated SSH over `AF_VSOCK` and Unix sockets. Notable because it gives a VM guest an SSH entry point with no sshd running and no port open. |

## Observability

| Component | Option | Wiring |
|---|---|---|
| `systemd-journald` | core | `/var/log/journal`, persistent. |
| `systemd-journal-remote`, `-upload`, `-gatewayd` | `-Dremote=enabled`, `-Dmicrohttpd=enabled` | Built and installed; units **not** enabled in the preset. Shipping a log-shipping daemon switched on by default would be a surprise. |
| `systemd-coredump`, `coredumpctl` | `-Dcoredump=true`, `-Delfutils=enabled` | Backtrace into the journal, via libdw. |
| `systemd-pstore` | `-Dpstore=true` | Pulls the previous crash out of firmware storage after a hard hang — the only record that survives a machine that never reached disk. |

## Containers, VMs and portable services

| Component | Option | Wiring |
|---|---|---|
| `systemd-nspawn` | `-Dnspawn=true` | |
| `systemd-vmspawn` | `-Dvmspawn=true` | |
| `systemd-machined`, `machinectl` | `-Dmachined=true` | In the preset. |
| `systemd-portabled`, `portablectl` | `-Dportabled=true` | Attach a service image to the host without a container runtime. In the preset. |
| `systemd-importd` | `-Dimportd=true`, `-Dlibcurl=enabled`, `-Dlibarchive=enabled` | |
| `systemd-dissect` | `-Dlibcryptsetup=enabled` | Inspects and mounts DDIs — the same image format sysext, portable services and vmspawn all use. |
| `systemd-storagetm` | `-Dstoragetm=true` | Exports the local disks over NVMe-TCP for recovery. In the preset: on a machine with no shell it is the recovery path. |

## Devices, resources, housekeeping

`systemd-udevd` + `udevadm` (core) · `systemd-hwdb` (`-Dhwdb=true`) ·
`systemd-modules-load` (core) · `systemd-binfmt` (`-Dbinfmt=true`) ·
`systemd-sysctl` · `systemd-vconsole-setup` (`-Dvconsole=true`) ·
`systemd-backlight` (`-Dbacklight=true`) · `systemd-rfkill` (`-Drfkill=true`) ·
`systemd-random-seed` (`-Drandomseed=true`) · `systemd-quotacheck`
(`-Dquotacheck=true`) · `systemd-fsck` · `systemd-remount-fs` ·
`systemd-hibernate-resume` + `systemd-sleep` (`-Dhibernate=true`) ·
`systemd-update-utmp`/`-update-done` · `systemd-volatile-root`.

Naming: `systemd-hostnamed`, `systemd-localed`, `systemd-timedated` and their
`*ctl` clients, all enabled, all used by GNOME's settings panels over D-Bus.

## Tooling

`systemctl` · `journalctl` · `systemd-analyze` (`-Danalyze=true`) ·
`systemd-creds` · `varlinkctl` · `busctl` · `systemd-cgls` · `systemd-cgtop` ·
`systemd-delta` · `systemd-detect-virt` · `systemd-escape` · `systemd-id128` ·
`systemd-path` · `systemd-mount` · `systemd-notify` · `systemd-run` ·
`systemd-socket-activate` · `systemd-stdio-bridge` · `systemd-ac-power` ·
`systemd-tty-ask-password-agent`.

---

## Deliberately off

Each of these is off for a reason, not by omission. Turning one on is a
decision, not a fix.

| Option | Why |
|---|---|
| `-Dselinux=disabled`, `-Dapparmor=disabled`, `-Dsmack=false` | No policy is shipped, and a MAC framework with no policy is a dependency with no benefit. The sandboxing this OS relies on is per-unit: `SystemCallFilter=` via seccomp, and `RestrictFileSystems=`/`SocketBind*=` via `-Dbpf-framework=enabled`. |
| `-Dutmp=false` | utmp is a fixed-size binary log from the 1980s. logind already tracks sessions, and `-Dsysv-compat=false` means nothing else reads it. |
| `-Dnscd=false` | Superseded by `nss-systemd` and resolved. |
| `-Dsysv-compat=false` | There are no init scripts here and never will be. |
| `-Dman=disabled`, `-Dhtml=disabled` | Would pull a full docbook toolchain into the build for output nobody reads on a machine with no shell. Restore both if you ever ship a terminal. |
| `-Dlibiptc=disabled` | iptables is superseded by nftables, and nothing in this OS writes firewall rules through systemd. |
| `-Dtests=false`, `-Dinstall-tests=false` | `pm build` runs `Test` steps (C11), so an upstream suite here would run on every build of every layer. The `inventory` step is the assertion that belongs at this level. |

## Conditional

`-Dbpf-framework=enabled` needs clang *and* `bpftool`, and `-Dtranslations=true`
needs `msgfmt`. Both reach the build through
`recipes/10-systemd/systemd/files/native.ini`, because pm cannot put a
pm-built program on `PATH` (C3). The recipe asserts all three exist before it
configures, so a missing one is one clear line rather than a meson probe failure
four hundred lines into a log. **If an injection cannot be made to work, turn
the option off rather than papering over it** — the `inventory` step will record
what was lost.
