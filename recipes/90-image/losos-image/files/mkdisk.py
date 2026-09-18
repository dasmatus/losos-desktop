#!/usr/bin/env python3
"""Assemble a GPT disk image from an ESP volume and a root filesystem.

This is what `losos.qcow2` is before `mkqcow2.py` wraps it, and it is the
layout the OS expects to wake up on: an EFI system partition carrying the UKI
and a root partition carrying the filesystem, both typed so that
`systemd-gpt-auto-generator` finds the root without an `/etc/fstab` and without
a `root=` on the kernel command line.

Nothing else is created. `/home` is deliberately absent: `systemd-repart` makes
it on first boot from `/usr/lib/repart.d/30-home.conf`, which is also what
marks it for factory reset, and a partition created here would be one repart
did not make and does not own. The disk is therefore larger than its
partitions, and the free space at the end is where repart works.

The image is sparse, so the free space costs nothing until something is written
into it -- and after `mkqcow2.py` it costs nothing in the shipped artifact
either.
"""

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import mkgpt  # noqa: E402

SECTOR = 512
# Partitions start on a megabyte boundary. Nothing here needs the alignment,
# but every partitioner a user might later point at this disk assumes it, and a
# disk that disagrees produces a warning that reads like corruption.
ALIGNMENT = 1 << 20
ALIGNMENT_SECTORS = ALIGNMENT // SECTOR


def align_up(value, to):
    return (value + to - 1) // to * to


def copy_into(image, source, offset):
    """Copy a partition image in at a byte offset, keeping its holes.

    A run of zeroes is seeked over rather than written, so a root filesystem
    that is mostly free space stays a hole in the disk image too. Without this
    a one-gigabyte filesystem holding fifty megabytes costs a gigabyte on the
    build host, in the artifact, and in every copy of it -- and the destination
    file has already been extended to its full length, so skipping a write
    leaves zeroes exactly as writing them would.
    """
    chunk_size = 1 << 20
    zeroes = b"\0" * chunk_size
    image.seek(offset)
    with open(source, "rb") as content:
        while True:
            chunk = content.read(chunk_size)
            if not chunk:
                break
            if len(chunk) == chunk_size and chunk == zeroes:
                image.seek(chunk_size, 1)
            else:
                image.write(chunk)


def build(esp, root, output, root_type, free_bytes, seed=""):
    esp_sectors = align_up(esp.stat().st_size, ALIGNMENT) // SECTOR
    root_sectors = align_up(root.stat().st_size, ALIGNMENT) // SECTOR

    esp_first = ALIGNMENT_SECTORS
    root_first = esp_first + esp_sectors
    end_of_partitions = root_first + root_sectors

    # Room for the backup table, then whatever free space was asked for, then
    # rounded up so the disk itself ends on a megabyte.
    disk_sectors = end_of_partitions + mkgpt.RESERVED_SECTORS
    disk_sectors += align_up(free_bytes, ALIGNMENT) // SECTOR
    disk_sectors = align_up(disk_sectors * SECTOR, ALIGNMENT) // SECTOR

    partitions = [
        mkgpt.Partition("esp", esp_first, esp_sectors, "ESP", seed=seed),
        mkgpt.Partition(root_type, root_first, root_sectors, "root", seed=seed),
    ]

    with open(output, "wb+") as image:
        image.truncate(disk_sectors * SECTOR)
        copy_into(image, esp, esp_first * SECTOR)
        copy_into(image, root, root_first * SECTOR)
        report = mkgpt.write(image, disk_sectors, partitions, seed=seed)

    report["bytes"] = disk_sectors * SECTOR
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--esp", required=True, help="the FAT volume to use as the ESP")
    parser.add_argument("--root", required=True, help="the root filesystem image")
    parser.add_argument("--output", required=True, help="the raw disk image to write")
    parser.add_argument("--root-type", default="root-x86-64",
                        help="GPT type of the root partition, by its systemd name")
    parser.add_argument("--free", type=int, default=8 << 30,
                        help="free space to leave after the last partition, "
                             "for systemd-repart to create /home in")
    parser.add_argument("--seed", default="",
                        help="string the derived partition GUIDs are a function of")
    args = parser.parse_args()

    esp, root = Path(args.esp), Path(args.root)
    for path in (esp, root):
        if not path.is_file():
            sys.exit(f"mkdisk: {path} does not exist")

    report = build(esp, root, args.output, args.root_type, args.free, args.seed)
    print(f"mkdisk: {report['bytes']} bytes, disk {report['disk_guid']}")
    for partition in report["partitions"]:
        print(f"mkdisk:   {partition['name']}: LBA {partition['first_lba']}"
              f"-{partition['last_lba']}, type {partition['type']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
