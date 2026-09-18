# The disk images, and why this repository writes filesystems itself

`./do build` ends with two whole disks — `losos.iso` and `losos.qcow2` — and a
root filesystem as a partition image. Producing them normally means `mkfs.ext4`,
`mkfs.vfat`, `xorriso` and `qemu-img`, four programs from four projects, plus a
loop device and root to drive them with. None of that is available here, and
the reasons are not incidental.

## Why not the usual tools

`mkfs`, `losetup`, `mount` and `dd` are not in pm's fingerprint table, and
`xorriso` and `qemu-img` are not either (C2 in
[`pm-constraints.md`](pm-constraints.md)). A command whose first word matches
nothing aborts the build before any step runs, so a recipe cannot invoke them
at all. Even if it could, pm's jail is a user namespace mapping one uid with a
minimal `/dev` (C7): there is no loop device to attach and no privilege to
mount anything on.

They could have been built from source into the sysroot and invoked through a
shell script, which is the shape `share/sysroot.sh` already uses. That was
considered and rejected for two reasons. It would add e2fsprogs, mtools and
libisoburn to the pinned source list — three more upstreams to track for a
build that otherwise needs a compiler and Python. And it would put
unfingerprinted programs behind a wrapper, which `limits.md` names as a
compromise this repository makes exactly once, deliberately, for `in-dir.sh`.

So the formats are written directly, in stdlib-only Python, by the same
argument that already produced `mkcpio.py` and `mkuki.py`: when the only
available tool cannot be run, writing the format is smaller than working around
not being able to run it.

## What writes what

All of these live in `recipes/90-image/losos-image/files/` and run as ordinary
`python3` steps, which is the one shape pm resolves without complaint (C3).

| Writer | Produces | Notable because |
|---|---|---|
| `mkext4.py` | the root filesystem | the largest piece of novel code in the tree |
| `mkfat.py` | the ESP | long filenames, which `losos-desktop_<version>_<arch>.efi` needs |
| `mkgpt.py` | the partition table | shared by both disks; types named as `architectures.yaml` names them |
| `mkdisk.py` | a raw GPT disk | ESP plus root, and free space for repart |
| `mkqcow2.py` | `losos.qcow2` | leaves zero clusters unallocated, so the artifact is the size of its content |
| `mkiso.py` | `losos.iso` | ISO 9660, El Torito and GPT over one copy of the bytes |

## The shape of the images

Both disks carry exactly two partitions: an EFI system partition and a root
partition, typed so `systemd-gpt-auto-generator` finds the root by GPT type —
which is why this OS ships no `/etc/fstab` and no `root=`. Everything else is
`systemd-repart`'s on first boot, from `/usr/lib/repart.d`: `/home`, the second
root partition an A/B update needs, and growing root into whatever disk it
landed on. A `/home` created at build time would be one repart did not make and
does not own, and a factory reset is *the absence of that partition* — so
creating it here would quietly break the reset path.

The two ESPs are not the same. The QCOW2's carries systemd-boot at the
removable-media path with the UKI beside it in `EFI/Linux/`, because an
installed system needs a loader to offer the previous kernel after a failed
update. The ISO's carries the installer UKI at that path directly: there is one
thing to boot, and a menu in front of it would only add a timeout.

The ISO is a hybrid. The same ESP image is pointed at by the El Torito boot
catalog (for firmware booting a DVD) and by a GPT partition entry (for firmware
booting a USB stick the file was written to with `dd`), and it is listed as a
file in the ISO 9660 directory as well. One copy of the bytes, three sets of
pointers — which is also what makes `CopyBlocks=auto` work from this medium:
the root partition the installer copies is a real partition on the device it
booted from.

## Reproducibility

Every image is a function of its inputs. Timestamps are the epoch, uid and gid
are flattened to 0 — both exactly as the rootfs tarball already did it — and
the identifiers that are normally random are derived instead: the ext4 UUID
from the tree, the FAT volume id from the tree, and every GPT GUID from the
version. Two builds of the same sources produce the same bytes; two different
builds still produce different identifiers, which is what anything looking one
up needs.

## How they are checked

`tools/gates/test-disk.py` builds a synthetic tree with one of everything — a
hard link, a fast symlink and a slow one, a directory that needs several
blocks, attributes that fit in an inode and attributes that do not, a file with
more extents than fit in an inode — and parses every result back with a reader
written from the format's own layout. A misunderstanding shared between a
writer and its own helpers round-trips perfectly; it does not round-trip
through a second implementation. That gate found a real bug before this was
ever committed: long filenames were being written with their chunks in the
right order and their ordinals in the wrong one, which produces a name that
reads back in reversed groups of thirteen characters and looks like truncation.

Where `e2fsck` is installed the gate runs it too and reports its verdict, but
it is never required: `./do check` has to pass on a machine with nothing but
Python.

`assert-media.py` runs inside the build, against the real artifacts, and asks
the smaller question the writers cannot ask about themselves — is the qcow2 a
qcow2 whose partition table holds an ESP with FAT in it and a root partition
with ext4 in it, and does the ISO's boot catalog point at the same bytes as its
ESP partition entry.

## What is still unproven

Reading a structure back is not booting it. `tools/vm-test disk` hands the
qcow2 to QEMU with UEFI firmware and requires the guest to report that it came
up, and that is the test that would settle it; it needs KVM and OVMF, which the
environment this was developed in has neither of. CI runs it.

One field is a known unknown and is written the way the other tools that meet
the same wall write it. El Torito's sector count is sixteen bits of 512-byte
sectors — 32 MiB — and a FAT32 volume cannot be smaller than about 34 MiB, so
an ESP large enough to hold a unified kernel image does not fit in that field.
`mkiso.py` writes zero there and relies on UEFI firmware reading the boot image
as a FAT volume and taking its size from the BPB, which is what the UEFI
specification describes. A firmware that instead honoured the field literally
would load nothing. Nobody has put this ISO in front of real firmware yet.
