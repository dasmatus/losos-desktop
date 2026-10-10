# Hibernation

A laptop hibernates: closing the lid suspends it, and after three hours, or
sooner if the battery runs low, it wakes, writes its memory to disk and
powers off. Opening it again brings back the same session, with every app as
it was. Hibernate is also in the command palette's session commands, where it
shows only on a machine that can do it.

Every other machine works as before: a desktop, a BIOS PC or a laptop without
a TPM suspends when the lid closes, and logind tells derisk it cannot
hibernate.

![Every boot, losos-hibernate-swap formats swap as LUKS2 with a new random key, seals it to the TPM's PCRs 4, 7 and 12 and forgets it; hibernating, systemd-sleep asks GRUB for the same UKI once and writes the image to the encrypted swap; resuming, the same UKI boots, the TPM unseals the key, and the kernel restores the image](images/hibernation.svg)

## Which machines

`losos-hibernate able` decides, early in every boot, before swap is
activated. A machine hibernates when both of these hold:

- **It is a laptop, a convertible or a tablet.** The chassis is read the way
  `systemd-hostnamed` reads it: `CHASSIS=` in `/etc/machine-info` first, then
  the SMBIOS chassis type, then the ACPI power profile, then the devicetree.
  Writing `CHASSIS=laptop` there (or `hostnamectl chassis laptop`) turns
  hibernation on for a machine the firmware describes wrongly, and
  `CHASSIS=desktop` turns it off.
- **systemd-stub measured the UKI into a TPM.** That is the variable
  `ConditionSecurity=measured-uki` reads. It means the machine has a TPM to
  seal the key to, and that it booted by UEFI, which systemd-sleep needs to
  record where the image is. A PC that boots by legacy BIOS has neither.

## The swap key

The swap partition (`losos-swap`, the size of the machine's RAM, [Where the
OS lives](layout.md)) has always been encrypted with a key nobody keeps:
crypttab maps it with a random key every boot, so what was swapped out is
unreadable once the machine powers off. A hibernation image needs a key that
outlives the power, and a laptop's swap gets one, sealed to the TPM:

1. **Boot.** `losos-hibernate-swap.service` formats the partition as LUKS2
   with 256 new random bits, opens it as `/dev/mapper/swap`, and has
   `systemd-cryptenroll` seal the key to the TPM's PCRs 4, 7 and 12. The key it
   formatted with is wiped from the header, and the script forgets it. Every
   boot does this, so, as before, nothing on the partition survives a boot
   that did not resume. If the TPM refuses, the swap falls back to the random
   key and the machine cannot hibernate until the next boot.
2. **Hibernate.** systemd-sleep writes the image to `/dev/mapper/swap` and
   records its location in the `HibernateLocation` EFI variable. The
   partition is the disk's only swap partition, so systemd marks it
   `autoSwap`. A sleep hook asks GRUB to start the version that is running
   once more (`losos_oneshot` in grubenv, [Boot loader](boot-loaders.md#boot-counting)),
   because sysupdate may have installed a newer one since this boot. On a
   disk installed before GRUB, which still starts systemd-boot, it asks that
   with `bootctl set-oneshot @current` instead.
3. **Resume.** In the initrd, `systemd-hibernate-resume-generator` reads the
   variable, opens `/dev/disk/by-designator/swap-luks` with the key the TPM
   unseals, and the kernel restores the image. That is all systemd's own; the
   image only changes the unlock to `headless`, so a refusal never stops at a
   passphrase prompt.

PCR 4 holds the boot loader and the UKI, so the TPM unseals the key only to
the UKI that sealed it: another OS started from a USB stick gets no key and no
image. PCR 12 holds what systemd-stub was handed besides the UKI. Without
Secure Boot (which the UKIs are not signed for, [What is not
done](not-done.md)) systemd-stub accepts a command line typed in the boot
menu, and one that starts a shell in place of the system changes PCR 12, so
it gets no key either. PCR 7, the Secure Boot state, is systemd's default.

## Lid, battery and timing

| When | What | Set in |
|---|---|---|
| Lid closes on battery | suspend, then hibernate after 3 h or at low battery | logind's `HandleLidSwitch=suspend-then-hibernate`, sleep.conf's `HibernateDelaySec=3h` |
| Lid closes on mains power | suspend | `HandleLidSwitchExternalPower=suspend` |
| Plugged in while suspended | the 3 hours stop counting | `HibernateOnACPower=no` |
| Battery critical while awake | hibernate | UPower's `CriticalPowerAction=Hibernate` |

On a machine that cannot hibernate, logind suspends where it would have
suspended and then hibernated, and UPower powers off where it would have
hibernated.

## What it does not do

- **A firmware update while hibernated.** Firmware that changes PCR 7 (a new
  Secure Boot database, for instance) or PCR 4 between hibernating
  and resuming leaves the TPM unwilling to unseal. The machine waits two
  minutes for the image, boots afresh, and the hibernated session is lost.
- **Choosing another boot entry on resume.** GRUB starts the UKI that
  hibernated unless someone picks a different one in its menu. A different
  one gets no key, as above.
- **BIOS PCs and laptops without a TPM** keep the per-boot key and do not
  hibernate. A BIOS has no EFI variable to record where an image is, and
  without a TPM the key would have to be on disk next to the image or typed
  in at every resume.
- **Phones.** The Halium image has its own power handling and no swap
  partition.

The test that proves it is `nix build .#checks.x86_64-linux.hibernate`: the
real image, a TPM, a machine that says it is a laptop, a hibernation, and a
resume into the same boot ID with a process and a file that only ever lived
in RAM still there.
