# Releases and GHCR

`nix build .#release` writes a flat directory:

```
losos-desktop_<v>_x86_64.efi                        the UKI sysupdate installs
losos-desktop_<v>_usr-x86-64_<uuid>.raw.xz          /usr for a sysupdate slot
losos-desktop_<v>_usr-x86-64-verity_<uuid>.raw.xz   its hash tree
losos-desktop_<v>.linux, .initrd, .grub             the UKI's kernel, initrd and
                                                    command line, for GRUB on a
                                                    BIOS PC, x86_64 only
losos-desktop_<v>_x86_64.raw.xz                     the disk image
losos-desktop_<v>_x86_64-installer.iso              the installer
losos-desktop_<v>_x86_64-windows-installer.exe      the installer for Windows, x86_64 only
losos-desktop_<v>_x86_64.qcow2                      the disk image, for a VM
SHA256SUMS
```

CI pushes it to GHCR as one OCI artifact per architecture with `oras`, and
`proxy/` serves the newest one to sysupdate, which cannot fetch from an OCI
registry itself (see [Binary cache](binary-cache.md)). The publish job then replaces the
`nightly` GitHub release with the installer ISOs, the
[Windows installer](windows-installer.md) and their lines of `SHA256SUMS`,
and nothing else: GitHub rejects release assets of 2 GiB or more, which
`/usr` and the disk image are not far from, and the installers are what a
person downloads by hand.

The `/usr` halves are cut out of the finished disk image at the offsets repart
reported, not built a second time, so the bytes sysupdate installs are the
bytes the image boots. Each name carries its partition's UUID, which sysupdate
reads with `@u` and gives the partition it writes. The initrd finds `/usr` by
the UUIDs repart derived from `usrhash=`, so a slot that kept the random UUID
repart created it with would hold the right bytes and never be found.
