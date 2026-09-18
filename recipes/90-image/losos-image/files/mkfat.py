#!/usr/bin/env python3
"""Write a FAT32 volume from a directory tree, because there is no mkfs.vfat.

The EFI system partition has to be FAT: firmware reads nothing else, and that
is a property of the machine rather than a choice this repository gets to make.
`mkfs.vfat` and `mcopy` are no more available inside pm's jail than `mkfs.ext4`
is (C2, C7), so the volume is written here, the same way the ext4 root is.

FAT32 rather than FAT16 or FAT12, and not only because the ESP is large: the
three are different formats sharing a boot sector, and picking one removes the
cluster-size table and the two other directory layouts that would otherwise
have to be right. UEFI firmware is required to read FAT32 on a hard disk, which
is what both the QCOW2 image and the hybrid ISO present.

Long names are emitted for anything that is not already a valid 8.3 name --
`losos-desktop_x86_64.efi` is not -- because systemd-boot and
systemd-sysupdate address kernels by their full names, and a volume that
silently truncated them would boot and then fail to update.

Deliberately stdlib-only, and deterministic: every timestamp is the start of
the FAT epoch and the volume id is derived from the tree, so two builds of the
same inputs produce the same bytes.
"""

import argparse
import hashlib
import os
import struct
import sys
from pathlib import Path

SECTOR = 512
RESERVED_SECTORS = 32
NUM_FATS = 2
# The smallest cluster count that is FAT32 rather than FAT16 wearing its boot
# sector. Below it, drivers are entitled to read the volume as FAT16 and find
# nothing.
MIN_CLUSTERS = 65525
MAX_CLUSTERS = 0x0FFFFFF5

ATTR_READ_ONLY = 0x01
ATTR_HIDDEN = 0x02
ATTR_SYSTEM = 0x04
ATTR_VOLUME_ID = 0x08
ATTR_DIRECTORY = 0x10
ATTR_ARCHIVE = 0x20
ATTR_LFN = 0x0F

END_OF_CHAIN = 0x0FFFFFFF
# 1980-01-01 00:00:00, the earliest a FAT timestamp can express. Fixed rather
# than taken from the source file so the volume is reproducible; nothing reads
# these, and a real time would change the image on every build.
FAT_DATE = (0 << 9) | (1 << 5) | 1
FAT_TIME = 0

# The characters a short name may contain. Everything else forces a long name,
# which is the common case here anyway.
SHORT_NAME_OK = set(
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789" "$%'-_@~`!(){}^#&"
)


def divide_up(value, by):
    return (value + by - 1) // by


class Entry:
    """A file or directory on the way to becoming directory entries."""

    def __init__(self, name, path, is_dir):
        self.name = name
        self.path = path
        self.is_dir = is_dir
        self.children = []
        self.size = 0 if is_dir else path.stat().st_size
        self.cluster = 0
        self.clusters = 0


def short_name(name, taken):
    """An 8.3 name for `name`, and whether a long name is needed beside it.

    A long name is needed whenever the short one is not simply the original
    uppercased -- which is almost always true here, because EFI filenames carry
    a version and an architecture.
    """
    upper = name.upper()
    base, _, extension = upper.rpartition(".")
    if not base:
        base, extension = upper, ""

    def clean(text):
        return "".join(c if c in SHORT_NAME_OK else "_" for c in text)

    stem = clean(base)[:8] or "_"
    suffix = clean(extension)[:3]
    exact = (
        upper == name
        and len(base) <= 8
        and len(extension) <= 3
        and all(c in SHORT_NAME_OK for c in base + extension)
    )

    candidate = f"{stem}.{suffix}" if suffix else stem
    if exact and candidate not in taken:
        taken.add(candidate)
        return candidate, False

    # A numeric tail, as every FAT implementation since MS-DOS 7 has done it.
    for index in range(1, 1000000):
        tail = f"~{index}"
        trimmed = stem[: 8 - len(tail)] + tail
        candidate = f"{trimmed}.{suffix}" if suffix else trimmed
        if candidate not in taken:
            taken.add(candidate)
            return candidate, True
    sys.exit(f"mkfat: cannot make a unique short name for {name}")


def pack_short(name):
    """The 11-byte on-disk form: eight of name, three of extension, padded.

    `.` and `..` are not 8.3 names and are not parsed as such: the dots go in
    the name field literally, which is why they are taken out before the split.
    """
    if name in (".", ".."):
        return name.ljust(11).encode("ascii")
    stem, _, suffix = name.partition(".")
    return (stem.ljust(8)[:8] + suffix.ljust(3)[:3]).encode("ascii")


def short_checksum(packed):
    """The checksum a long-name entry carries to tie it to its short entry."""
    total = 0
    for byte in packed:
        total = (((total & 1) << 7) + (total >> 1) + byte) & 0xFF
    return total


def long_name_entries(name, checksum):
    """The long-name entries for `name`, in the order they go on disk.

    Thirteen UTF-16 code units each. The ordinal counts the chunks forwards,
    from the one holding the first thirteen characters, but they are written to
    disk backwards: the highest ordinal first, carrying the flag that marks it
    last, and ordinal 1 immediately before the short entry it belongs to.
    Getting those two orders the same way round produces a name that reads back
    reversed in chunks of thirteen, which looks like a truncation rather than
    like a byte-order mistake.
    """
    encoded = name.encode("utf-16-le")
    units = [encoded[i:i + 2] for i in range(0, len(encoded), 2)]
    units.append(b"\0\0")
    while len(units) % 13:
        units.append(b"\xff\xff")

    chunks = [units[i:i + 13] for i in range(0, len(units), 13)]
    entries = []
    for ordinal in range(len(chunks), 0, -1):
        body = b"".join(chunks[ordinal - 1])
        marked = ordinal | (0x40 if ordinal == len(chunks) else 0)
        entries.append(
            struct.pack("<B", marked)
            + body[0:10]
            + struct.pack("<BBB", ATTR_LFN, 0, checksum)
            + body[10:22]
            + struct.pack("<H", 0)
            + body[22:26]
        )
    return entries


class FatBuilder:
    def __init__(self, root, label="", size=None, volume_id=None):
        self.root = Path(root).resolve()
        self.label = label.upper()[:11]
        self.requested_size = size
        self.volume_id = volume_id

    def scan(self, path, name=""):
        entry = Entry(name, path, True)
        for child in sorted(os.scandir(path), key=lambda e: e.name):
            if child.is_symlink():
                # FAT has no symlinks and the ESP needs none: everything on it
                # is a file firmware opens by path.
                sys.exit(f"mkfat: {child.path} is a symlink, which FAT cannot hold")
            if child.is_dir():
                entry.children.append(self.scan(Path(child.path), child.name))
            elif child.is_file():
                entry.children.append(Entry(child.name, Path(child.path), False))
            else:
                sys.exit(f"mkfat: {child.path} is neither a file nor a directory")
        return entry

    def directory_entry_count(self, entry, is_root):
        count = 0 if is_root else 2  # "." and ".."
        if is_root and self.label:
            count += 1  # the volume label lives in the root directory
        taken = set()
        for child in entry.children:
            _, needs_long = short_name(child.name, taken)
            count += 1 + (len(long_name_entries(child.name, 0)) if needs_long else 0)
        return count

    def measure(self, entry, is_root=False):
        """Cluster counts for every node, bottom up."""
        if entry.is_dir:
            entries = self.directory_entry_count(entry, is_root)
            entry.size = entries * 32
            entry.clusters = max(1, divide_up(entry.size, self.cluster_bytes))
            for child in entry.children:
                self.measure(child)
        else:
            entry.clusters = divide_up(entry.size, self.cluster_bytes)

    def total_clusters_needed(self, entry):
        total = entry.clusters
        for child in entry.children:
            total += self.total_clusters_needed(child)
        return total

    def geometry(self, total_sectors, sectors_per_cluster):
        """FAT size and cluster count for a volume of this size.

        Circular, like every FAT geometry: the FAT is as big as the number of
        clusters requires, and the number of clusters is what is left after the
        FAT. Two rounds settle it.
        """
        fat_sectors = 1
        for _ in range(8):
            data = total_sectors - RESERVED_SECTORS - NUM_FATS * fat_sectors
            if data <= 0:
                return None
            clusters = data // sectors_per_cluster
            needed = divide_up((clusters + 2) * 4, SECTOR)
            if needed == fat_sectors:
                return fat_sectors, clusters
            fat_sectors = needed
        return None

    def choose_geometry(self, size_bytes):
        total_sectors = size_bytes // SECTOR
        for sectors_per_cluster in (1, 2, 4, 8, 16, 32, 64, 128):
            found = self.geometry(total_sectors, sectors_per_cluster)
            if not found:
                continue
            fat_sectors, clusters = found
            if clusters < MIN_CLUSTERS or clusters > MAX_CLUSTERS:
                continue
            # A FAT of a few hundred kilobytes is the point at which a bigger
            # cluster is worth the slack it wastes.
            if clusters <= 1 << 18:
                return sectors_per_cluster, fat_sectors, clusters, total_sectors
        sys.exit(
            f"mkfat: no FAT32 geometry fits {size_bytes} bytes. A FAT32 volume "
            f"needs at least {MIN_CLUSTERS} clusters, so about 34 MiB at the "
            "smallest cluster size."
        )

    # ------------------------------------------------------------ allocation

    def allocate(self, entry, is_root=False):
        """Hand each node a contiguous cluster chain, parents before children.

        Contiguous because nothing is ever deleted from this volume, and in
        depth-first order so a directory's own cluster is known before its
        entries are written.
        """
        entry.cluster = self.next_cluster
        self.next_cluster += entry.clusters
        if self.next_cluster - 2 > self.clusters:
            sys.exit("mkfat: the tree does not fit in the volume")
        for child in entry.children:
            if child.clusters:
                self.allocate(child)
            else:
                # An empty file occupies no clusters and says so with a first
                # cluster of zero, which is how every FAT driver reads "empty".
                child.cluster = 0

    # --------------------------------------------------------------- writing

    def directory_bytes(self, entry, parent_cluster, is_root):
        data = bytearray()
        if is_root and self.label:
            # The label is a raw eleven-byte field rather than a name that
            # happens to be eleven bytes: it is not split at a dot.
            data += (
                self.label.ljust(11)[:11].encode("ascii")
                + struct.pack("<BBBHHHHHHHI", ATTR_VOLUME_ID, 0, 0, FAT_TIME,
                              FAT_DATE, FAT_DATE, 0, FAT_TIME, FAT_DATE, 0, 0)
            )
        if not is_root:
            data += self.entry_bytes(".", ATTR_DIRECTORY, entry.cluster, 0)
            # A ".." pointing at the root is stored as cluster 0, not as 2.
            data += self.entry_bytes("..", ATTR_DIRECTORY, parent_cluster, 0)

        taken = set()
        for child in entry.children:
            name, needs_long = short_name(child.name, taken)
            packed = pack_short(name)
            if needs_long:
                for long_entry in long_name_entries(child.name,
                                                    short_checksum(packed)):
                    data += long_entry
            attributes = ATTR_DIRECTORY if child.is_dir else ATTR_ARCHIVE
            data += self.entry_bytes(name, attributes, child.cluster,
                                     0 if child.is_dir else child.size)
        return bytes(data)

    def entry_bytes(self, name, attributes, cluster, size):
        return (
            pack_short(name)
            + struct.pack(
                "<BBBHHHHHHHI",
                attributes,
                0,                       # reserved / lowercase hints, unused
                0,                       # creation time, tenths
                FAT_TIME, FAT_DATE,      # creation
                FAT_DATE,                # last access
                (cluster >> 16) & 0xFFFF,
                FAT_TIME, FAT_DATE,      # last write
                cluster & 0xFFFF,
                size,
            )
        )

    def write_chain(self, image, entry, parent_cluster, is_root):
        if entry.is_dir:
            payload = self.directory_bytes(entry, parent_cluster, is_root)
            payload += b"\0" * (entry.clusters * self.cluster_bytes - len(payload))
            self.put_clusters(image, entry.cluster, payload)
            for child in entry.children:
                self.write_chain(image, child, entry.cluster, False)
        elif entry.clusters:
            image.seek(self.cluster_offset(entry.cluster))
            with open(entry.path, "rb") as source:
                remaining = entry.size
                while remaining > 0:
                    chunk = source.read(min(1 << 20, remaining))
                    if not chunk:
                        sys.exit(f"mkfat: {entry.path} shrank while reading")
                    image.write(chunk)
                    remaining -= len(chunk)
            tail = (-entry.size) % self.cluster_bytes
            if tail:
                image.write(b"\0" * tail)
        self.chain(entry)

    def chain(self, entry):
        """Link this node's clusters together in the FAT."""
        for index in range(entry.clusters):
            cluster = entry.cluster + index
            last = index == entry.clusters - 1
            self.fat[cluster] = END_OF_CHAIN if last else cluster + 1

    def cluster_offset(self, cluster):
        return (self.data_start + (cluster - 2) * self.sectors_per_cluster) * SECTOR

    def put_clusters(self, image, cluster, payload):
        image.seek(self.cluster_offset(cluster))
        image.write(payload)

    def boot_sector(self, total_sectors):
        raw = bytearray(SECTOR)
        raw[0:3] = b"\xeb\x58\x90"
        # The OEM name every firmware has been tested against. It is not a
        # label; some implementations have historically keyed behaviour off it.
        raw[3:11] = b"MSWIN4.1"
        struct.pack_into(
            "<HBHBHHBHHHII", raw, 11,
            SECTOR,
            self.sectors_per_cluster,
            RESERVED_SECTORS,
            NUM_FATS,
            0,                      # root entry count: zero on FAT32
            0,                      # total sectors 16: zero, see below
            0xF8,                   # media descriptor: a fixed disk
            0,                      # FAT size 16: zero on FAT32
            32, 64,                 # geometry nothing reads, but which must
            0,                      # hidden sectors
            total_sectors,
        )
        struct.pack_into(
            "<IHHIHH", raw, 36,
            self.fat_sectors,
            0,                      # ext flags: both FATs live, FAT 0 active
            0,                      # filesystem version
            2,                      # the root directory's first cluster
            1,                      # FSInfo sector
            6,                      # backup boot sector
        )
        struct.pack_into("<BBB", raw, 64, 0x80, 0, 0x29)
        struct.pack_into("<I", raw, 67, self.volume_id)
        raw[71:82] = (self.label or "NO NAME").ljust(11)[:11].encode("ascii")
        raw[82:90] = b"FAT32   "
        # No boot code: this volume is read by firmware, which looks at the
        # BPB and the directory, never at the first sector's instructions.
        struct.pack_into("<H", raw, 510, 0xAA55)
        return bytes(raw)

    def fsinfo_sector(self, free_clusters, next_free):
        raw = bytearray(SECTOR)
        struct.pack_into("<I", raw, 0, 0x41615252)
        struct.pack_into("<I", raw, 484, 0x61417272)
        struct.pack_into("<II", raw, 488, free_clusters, next_free)
        struct.pack_into("<I", raw, 508, 0xAA550000)
        return bytes(raw)

    def build(self, output):
        tree = self.scan(self.root)

        # Two passes over the size: the tree's cluster count depends on the
        # cluster size, and the cluster size depends on the volume's.
        size = self.requested_size
        if size is None:
            self.cluster_bytes = 4096
            self.measure(tree, is_root=True)
            needed = self.total_clusters_needed(tree) * 4096
            size = max(needed * 5 // 4, (MIN_CLUSTERS + 64) * SECTOR)
            size = divide_up(size, 1 << 20) * (1 << 20)

        (self.sectors_per_cluster, self.fat_sectors, self.clusters,
         total_sectors) = self.choose_geometry(size)
        self.cluster_bytes = self.sectors_per_cluster * SECTOR
        self.data_start = RESERVED_SECTORS + NUM_FATS * self.fat_sectors

        self.measure(tree, is_root=True)
        self.volume_id = self.volume_id or self.derive_volume_id(tree)

        self.fat = [0] * (self.clusters + 2)
        self.fat[0] = 0x0FFFFFF8
        self.fat[1] = END_OF_CHAIN
        self.next_cluster = 2

        self.allocate(tree, is_root=True)

        with open(output, "wb+") as image:
            image.truncate(total_sectors * SECTOR)
            self.write_chain(image, tree, 0, True)

            table = bytearray(self.fat_sectors * SECTOR)
            for cluster, value in enumerate(self.fat):
                struct.pack_into("<I", table, cluster * 4, value)
            for copy in range(NUM_FATS):
                image.seek((RESERVED_SECTORS + copy * self.fat_sectors) * SECTOR)
                image.write(table)

            used = self.next_cluster - 2
            boot = self.boot_sector(total_sectors)
            fsinfo = self.fsinfo_sector(self.clusters - used, self.next_cluster)
            for base in (0, 6):
                image.seek(base * SECTOR)
                image.write(boot)
                image.write(fsinfo)

        return {
            "bytes": total_sectors * SECTOR,
            "cluster_bytes": self.cluster_bytes,
            "clusters": self.clusters,
            "used": used,
            "volume_id": f"{self.volume_id:08X}",
        }

    def derive_volume_id(self, tree):
        """A volume id that is a function of the tree rather than of the clock."""
        digest = hashlib.sha256()
        digest.update(b"losos-desktop/fat32/1\0")

        def walk(entry, prefix):
            digest.update(f"{prefix}/{entry.name}:{entry.size}\0".encode())
            for child in entry.children:
                walk(child, prefix + "/" + entry.name)

        walk(tree, "")
        return struct.unpack("<I", digest.digest()[:4])[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("root", help="directory tree to put in the volume")
    parser.add_argument("output", help="image file to write")
    parser.add_argument("--label", default="", help="volume label, up to 11 bytes")
    parser.add_argument("--size", type=int, help="volume size in bytes")
    args = parser.parse_args()

    root = Path(args.root)
    if not root.is_dir():
        sys.exit(f"mkfat: {root} is not a directory")

    report = FatBuilder(root, label=args.label, size=args.size).build(args.output)
    print(
        "mkfat: {bytes} bytes, {clusters} clusters of {cluster_bytes}, "
        "{used} used, id {volume_id}".format(**report)
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
