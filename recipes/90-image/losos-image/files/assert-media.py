#!/usr/bin/env python3
"""Check the disk images structurally, inside the build that produced them.

`tools/gates/test-disk.py` proves the writers against synthetic trees and runs
anywhere. This runs against the real artifacts, in the jail, as a `Test` step,
and asks the smaller question those writers cannot answer about themselves: is
what came out actually a qcow2, an ISO with a boot catalog, and a partition
table whose partitions hold the filesystems they claim to.

Every failure here is one that firmware reports as a machine that powers on and
sits at a boot menu with nothing in it.

Deliberately duplicates nothing from the writers: it reads magic numbers and
offsets out of the finished files.
"""

import argparse
import struct
import sys
from pathlib import Path

SECTOR = 512
ISO_SECTOR = 2048


def fail(message):
    print(f"assert-media: {message}", file=sys.stderr)
    return False


class Qcow2View:
    """Enough of a qcow2 reader to look at the disk inside one.

    Only the mapping: header, L1, L2, and zeroes for anything unallocated.
    Reading the partition table out of the qcow2 itself is the point -- a check
    against the raw disk it was made from would pass even if the conversion had
    dropped every cluster.
    """

    def __init__(self, path):
        self.image = open(path, "rb")
        header = self.image.read(104)
        if header[0:4] != b"QFI\xfb":
            raise ValueError("not a qcow2 file")
        self.version = struct.unpack_from(">I", header, 4)[0]
        self.cluster_bits = struct.unpack_from(">I", header, 20)[0]
        self.size = struct.unpack_from(">Q", header, 24)[0]
        self.l1_size = struct.unpack_from(">I", header, 36)[0]
        self.l1_offset = struct.unpack_from(">Q", header, 40)[0]
        self.cluster = 1 << self.cluster_bits
        self.l2_entries = self.cluster // 8
        self.position = 0

    def seek(self, offset):
        self.position = offset

    def read(self, length):
        out = bytearray()
        while len(out) < length:
            index, inside = divmod(self.position, self.cluster)
            take = min(self.cluster - inside, length - len(out))
            out += self._cluster(index)[inside:inside + take]
            self.position += take
        return bytes(out)

    def _cluster(self, index):
        l1_index, l2_index = divmod(index, self.l2_entries)
        if l1_index >= self.l1_size:
            return bytes(self.cluster)
        self.image.seek(self.l1_offset + l1_index * 8)
        l2_offset = struct.unpack(">Q", self.image.read(8))[0] & 0x00FFFFFFFFFFFE00
        if not l2_offset:
            return bytes(self.cluster)
        self.image.seek(l2_offset + l2_index * 8)
        offset = struct.unpack(">Q", self.image.read(8))[0] & 0x00FFFFFFFFFFFE00
        if not offset:
            return bytes(self.cluster)
        self.image.seek(offset)
        return self.image.read(self.cluster).ljust(self.cluster, b"\0")


def partitions(image):
    """The partition table's entries, read from the primary header."""
    image.seek(SECTOR)
    header = image.read(92)
    if header[0:8] != b"EFI PART":
        return None
    entries_lba, count, size, _ = struct.unpack_from("<QIII", header, 72)
    image.seek(entries_lba * SECTOR)
    body = image.read(count * size)
    found = []
    for index in range(count):
        entry = body[index * size:(index + 1) * size]
        if entry[0:16] == bytes(16):
            continue
        first, last, _ = struct.unpack_from("<QQQ", entry, 32)
        found.append((entry[0:16], first, last,
                      entry[56:128].decode("utf-16-le").rstrip("\0")))
    return found


def check_disk(image, what):
    """A GPT with an ESP holding FAT32 and a root partition holding ext4."""
    table = partitions(image)
    if not table:
        return fail(f"{what} has no GPT header at LBA 1")
    if len(table) != 2:
        return fail(f"{what} has {len(table)} partitions, expected 2")

    esp, root = table[0], table[1]
    image.seek(esp[1] * SECTOR)
    boot = image.read(512)
    if boot[82:90] != b"FAT32   ":
        return fail(f"{what}: the ESP does not begin with a FAT32 volume")
    if boot[510:512] != b"\x55\xaa":
        return fail(f"{what}: the ESP has no boot sector signature")

    image.seek(root[1] * SECTOR + 1024)
    superblock = image.read(1024)
    if struct.unpack_from("<H", superblock, 0x38)[0] != 0xEF53:
        return fail(f"{what}: the root partition does not begin with an ext4 "
                    "superblock")
    return True


def check_qcow2(path):
    try:
        view = Qcow2View(path)
    except (ValueError, OSError) as problem:
        return fail(f"{path}: {problem}")
    if view.version != 3:
        return fail(f"{path} is qcow2 version {view.version}, expected 3")
    if view.size == 0:
        return fail(f"{path} declares a zero-byte disk")
    if not check_disk(view, path):
        return False
    print(f"assert-media: {path} is qcow2 v3 over a {view.size} byte disk, "
          "with an ESP and a root partition in it")
    return True


def check_iso(path):
    with open(path, "rb") as image:
        image.seek(16 * ISO_SECTOR)
        primary = image.read(ISO_SECTOR)
        if primary[0] != 1 or primary[1:6] != b"CD001":
            return fail(f"{path} has no ISO 9660 primary volume descriptor")

        image.seek(17 * ISO_SECTOR)
        boot = image.read(ISO_SECTOR)
        if boot[0] != 0 or boot[7:30] != b"EL TORITO SPECIFICATION":
            return fail(f"{path} has no El Torito boot record")
        catalog_sector = struct.unpack_from("<I", boot, 71)[0]

        image.seek(catalog_sector * ISO_SECTOR)
        catalog = image.read(64)
        if catalog[1] != 0xEF:
            return fail(f"{path}: the boot catalog is for platform "
                        f"{catalog[1]:#x}, not UEFI")
        if catalog[32] != 0x88:
            return fail(f"{path}: the boot entry is not marked bootable")
        boot_extent = struct.unpack_from("<I", catalog, 40)[0]

        # The one thing that makes this a hybrid rather than two images in a
        # trench coat: the El Torito boot image and the GPT's ESP have to be
        # the same bytes. If they drift, the DVD boots one kernel and the USB
        # stick boots another.
        table = partitions(image)
        if not table:
            return fail(f"{path} has no GPT, so it would not boot from USB")
        if boot_extent * ISO_SECTOR != table[0][1] * SECTOR:
            return fail(f"{path}: the boot catalog points at byte "
                        f"{boot_extent * ISO_SECTOR} and the ESP partition "
                        f"starts at {table[0][1] * SECTOR}")

        if not check_disk(image, path):
            return False

    print(f"assert-media: {path} is a hybrid ISO; El Torito and the GPT agree")
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--qcow2", required=True)
    parser.add_argument("--iso", required=True)
    args = parser.parse_args()

    ok = check_qcow2(Path(args.qcow2))
    ok = check_iso(Path(args.iso)) and ok
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
