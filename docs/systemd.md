# Everything systemd, and the exceptions

Which systemd component does which job:

| Job | Component |
|---|---|
| Boot | `systemd-boot`, `systemd-stub` (UKI), `bless-boot`, `boot-check-no-failures`; GRUB on a legacy BIOS PC |
| Installation | a live ISO runs `derisk installer` as its session, with `wpa_supplicant` for Wi-Fi; its backend runs `systemd-repart` for the ESP and slot A, and `systemd-sysupdate` to fill them from the channel |
| First boot | `systemd-repart` in the initrd creates slot B, root, `/home` and swap; `derisk setup` asks for a language, keyboard, time zone, network and the first user, saved through localed, timedated and homed |
| Read-only `/usr` | the Nix store on a dm-verity partition, with its root hash on the UKI's command line; `systemd-sysext` and `systemd-confext` add layers on top |
| Updates | `systemd-sysupdate` with A/B slots, and `fwupd` for firmware |
| Factory reset | `FactoryReset=` on root and `/home`, and systemd's reset Varlink API |
| Accounts | `systemd-homed`, `systemd-userdbd`, `pam_systemd_home`, `run0` |
| Session | derisk as the display manager, with its lock screen as the login screen, `systemd-logind` seats, and the user manager driving `derisk-session.target` and `graphical-session.target` |
| Network | `systemd-networkd`, `systemd-resolved`, `systemd-timesyncd` |
| Memory | `systemd-oomd`, zswap in front of a RAM-sized encrypted swap partition, and GrapheneOS's hardened_malloc as every process's allocator on PCs |

![A machine's life: the installer lays out the disk with systemd-repart and fills slot A with systemd-sysupdate; first boot creates slot B, root, home and swap; derisk setup creates the first account with homectl; homed opens each person's home at login; sysupdate writes the idle slot and boot counting keeps or gives up the new version; a factory reset empties root and home](images/systemd.svg)

The places a
non-systemd component remains are the places systemd has no equivalent, or
NixOS requires one:

- **nsncd.** The `homed` module asserts `services.nscd.enable`: NixOS routes
  NSS through a forwarder so a libc can find `nss-systemd` in the store. It is
  stateless and holds no idea of its own about who exists. glibc's NSS uses it
  to find `nss-systemd`.
- **nftables.** systemd has no packet filter. The pm tree shipped none at all;
  this keeps NixOS's firewall and opens mDNS and LLMNR for resolved.
- **A PAM stack.** The pm tree shipped none, and `docs/limits.md` said nothing
  in the image could authenticate anyone. NixOS generates one, with
  `pam_systemd_home` in all four management groups -- the condition for a
  login to open the home area rather than succeed and find it locked.

Replaced by the systemd equivalent where NixOS would otherwise pick something
else: `run0` instead of sudo (with a `sudo` alias that refuses sudo's flags);
networkd instead of NetworkManager, which GNOME turns on by default; resolved's
mDNS instead of avahi; `systemd-sysusers` instead of the perl user script; the
`/etc` overlay instead of activation scripts writing files; first-boot setup
making the first user with homectl instead of no way to create one at all.
