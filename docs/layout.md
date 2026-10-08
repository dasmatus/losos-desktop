# Where the OS lives

The one structural change is where the operating system is.

In the pm tree it was the root partition: `/usr` and `/etc` together, replaced
wholesale by sysupdate, with `/home` beside it. Here the operating system is
the Nix store, on its own `/usr` partition, protected by dm-verity, and the
root partition holds only state: `/etc`'s writable layer, `/var`, the journal.

```
BIOS boot    GRUB's core, for a legacy BIOS (x86_64)  image
ESP          systemd-boot, the UKIs                  image
usr-verity A dm-verity hash tree for slot A          image
usr A        the Nix store, erofs                    image, grown on first boot
usr-verity B                                         first boot, label _empty
usr B        where the next update is written        first boot, label _empty
root         state only                              first boot, FactoryReset=yes
home         homed's LUKS images                     first boot, FactoryReset=yes
swap         RAM-sized, a new key each boot          first boot (losos-swap)
```

![The disk from left to right: BIOS boot, ESP, usr-verity A and usr A come from the image; usr-verity B, usr B, root, home and swap are created on the first boot. The UKI's usrhash= names slot A's verity tree, sysupdate writes whichever slot is not in use, and a factory reset empties root and home](images/layout.svg)

On x86_64, GRUB's boot code is also in the MBR, so a PC without UEFI starts
the same disk ([Boot loaders](boot-loaders.md)).

Swap's key is random on most machines and kept by nobody. A laptop with a TPM
seals it to the TPM instead, so the partition can hold a hibernation image
([Hibernation](hibernation.md)); either way it is new every boot.

That buys two things the pm tree wrote down as limits:

- **`/usr` is immutable for real.** The pm tree's boot documentation named
  `systemd-veritysetup` in the boot chain but never built a verity partition.
  Here the root hash is `usrhash=` on the UKI's command line, so a changed
  block in `/usr` fails verification, and a different `/usr` needs a
  different UKI.
- **A factory reset resets the machine.** The pm tree's factory-reset notes explained
  that marking only `/home` left `/etc` and `/var` behind, and that the fix was
  to get state off the partition the OS lives on. That is now the layout, so
  root is marked too, and a reset returns the machine to exactly what the image
  contained.
