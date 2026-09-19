# The disk images

`./do build` ends with two whole disks — `losos.iso` and `losos.qcow2` — and
the root filesystem as a partition image. All three come out of one tree that
pm has already built, and out of three programs the build host provides:
`mkosi`, `xorriso` and `qemu-img`.

## Why these tools, when pm's table refuses them

`mkosi`, `xorriso` and `qemu-img` are all absent from pm's fingerprint table,
and a command whose first word matches nothing aborts the build before any step
runs (C2 in [`pm-constraints.md`](pm-constraints.md)). That refusal is not pm
being difficult: the table is what makes `pm explain` a real answer to "what
does this build need", and a table with a permissive fallback would answer
nothing.

A plugin is how a distribution extends the table without weakening it. pm only
ever consults one about a command that matched **no** built-in fingerprint, so
`plugins/losos-mkosi` can name `mkosi` and can never reclassify `cargo` as
needing no network. `plugins/losos-image` does the same for `xorriso`,
`qemu-img` and the filesystem builders. Both declare a ceiling that excludes
`Network`, which matters more here than elsewhere: network in a pm build file
is per *file* rather than per step (C8), so one careless grant would put the
whole image layer's jail on the host network.

This tree used to write ext4, FAT32, GPT, ISO 9660 and qcow2 itself, in
stdlib-only Python, and the argument for that was the paragraph above read
backwards — that pm would refuse the real tools. It was wrong: the
`losos-image` plugin already named them. What was actually missing was any
guarantee that they were *installed* on whatever host ran the build, and that
is what [`container.md`](container.md) fixes. The writers are gone; their
history is in git.

## What runs, in order

| Step | Tool | Produces |
|---|---|---|
| `esp-tree` | `cp` | the ESP's contents, inside the tree at `/efi` |
| `disk` | mkosi | `losos-disk.raw`, and its partitions as separate files |
| `qcow2` | `qemu-img` | `losos.qcow2` |
| `installer-esp` | mkosi | a bare ESP carrying the installer UKI |
| `iso` | `xorriso` | `losos.iso` |
| `root-image-xz` | `xz` | `losos-root.raw.xz` |

mkosi is not installing a distribution. `Distribution=custom` tells it there is
no package manager to drive; `BaseTrees=` hands it the tree pm produced. What
is left is the part mkosi is better at than anything else available: driving
`systemd-repart` to lay out partitions and populate filesystems.

**`RepartOffline=yes` is the setting that makes this possible at all.** Offline,
repart populates a filesystem through `mkfs`' own populate modes rather than by
attaching a loop device and mounting it — and a loop device is exactly what
pm's jail has no `/dev` node and no privilege for (C7).

**`SplitArtifacts=partitions` is what keeps the root filesystem a first-class
artifact** rather than something trapped inside a disk image. systemd-sysupdate
writes it into the inactive root partition and the installer duplicates it with
`CopyBlocks=auto`; both take a partition, not a disk.

## The shape of the images

Both disks carry an EFI system partition and a root partition, typed so
`systemd-gpt-auto-generator` finds the root by GPT type — which is why this OS
ships no `/etc/fstab` and no `root=`. Everything else is `systemd-repart`'s on
first boot, from `/usr/lib/repart.d`: `/home`, the second root partition an A/B
update needs, and growing root into whatever disk it landed on. A `/home`
created at build time would be one repart did not make and does not own, and a
factory reset is *the absence of that partition* — so creating it here would
quietly break the reset path.

The qcow2's virtual size is grown by twelve gigabytes after the conversion
rather than the raw image being padded before it. That is where repart works on
first boot, it costs nothing in the artifact because an unallocated cluster is
not stored, and it leaves the GPT backup header short of the new end — which
repart relocates on its first run, exactly as it does for every cloud image
shipped this way.

The two ESPs are not the same. The disk's carries systemd-boot at the
removable-media path with the UKI beside it in `EFI/Linux`, because an installed
system needs a loader to offer the previous kernel after a failed update. The
ISO's carries the installer UKI at that path directly: there is one thing to
boot, and a menu in front of it would only add a timeout.

The ESP's contents are staged into the tree at `/efi`, where they are mounted on
a running system, and the root partition's definition excludes that path. Without
the exclusion every unified kernel image would be in the image twice — once
where firmware reads it and once inside the root filesystem where nothing ever
will — and the root partition image shipped for updates would carry a copy of
the kernel it is shipped alongside.

## The ISO is a hybrid, over one copy of the bytes

`xorriso -append_partition` puts the installer ESP and the root partition into
the ISO's GPT, and `-e --interval:appended_partition_2:all::` points the El
Torito boot entry at the appended ESP itself rather than at a second copy inside
the ISO 9660 filesystem. Firmware booting a DVD reads the boot catalog;
firmware booting a USB stick the file was `dd`'d to reads the GPT; both land on
the same bytes. If they ever drifted, the DVD would boot one kernel and the USB
stick another.

The root partition is on the medium because that is what the installer
installs: `CopyBlocks=auto` copies the partition it booted from.

## Reproducibility

Every image is a function of its inputs. `SourceDateEpoch=0` and `Seed=` fix the
two things that are otherwise generated fresh on every run — timestamps and
every partition UUID. The seed is derived by `tools/configure` from the
architecture and the version, so two builds of one version agree and two
versions differ, which is what anything looking a partition up needs.

## How they are checked

`assert-media.py` runs inside the build, against the real artifacts, as a `Test`
step. It asks the question the tools cannot ask about themselves: is the qcow2 a
qcow2 whose partition table holds an ESP with FAT in it and a discoverable root
with ext4 in it, and does the ISO's boot catalog point at the same bytes as its
ESP partition entry. Partitions are looked up by type GUID rather than by
position, because xorriso puts the ISO 9660 image area in the table too.

Reading a structure back is not booting it. `tools/vm-test disk` hands the qcow2
to QEMU with UEFI firmware and requires the guest to report that it came up, and
that is the test that settles it; CI runs it on both architectures.
`tools/libvirt-domain` writes the same machine for virt-manager, for looking at
the desktop rather than asserting it exists.
