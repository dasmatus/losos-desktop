#!/usr/bin/env python3
"""Convert a raw disk image to qcow2, because there is no qemu-img.

`qemu-img` is not in pm's fingerprint table and is not installed on the build
hosts this repository targets (C2, C3), and requiring it would put a
virtualisation package in the dependency list of a build that otherwise needs a
compiler and Python. qcow2 is a simple enough container that writing it is
smaller than depending on something that can.

Only what a freshly written image needs: version 3, no backing file, no
encryption, no snapshots, no compression. Clusters that are entirely zero in
the raw image are left unallocated rather than written out, so a twenty-gigabyte
disk holding a two-gigabyte filesystem is a two-gigabyte file -- which is the
practical reason to ship qcow2 at all rather than the raw disk.

Everything is big-endian, which is the one thing about the format that
surprises people writing it for the first time.
"""

import argparse
import struct
import sys
from pathlib import Path

MAGIC = b"QFI\xfb"
VERSION = 3
HEADER_LENGTH = 104
# 64KiB clusters: the qemu default, and what every reader is tuned for.
CLUSTER_BITS = 16
CLUSTER = 1 << CLUSTER_BITS
L2_ENTRIES = CLUSTER // 8
# refcount_order 4 means 16-bit refcounts, which is also the qemu default.
REFCOUNT_ORDER = 4
REFCOUNT_BITS = 1 << REFCOUNT_ORDER
REFCOUNTS_PER_BLOCK = CLUSTER * 8 // REFCOUNT_BITS
REFCOUNT_TABLE_ENTRIES = CLUSTER // 8

# Set on an L1 or L2 entry whose cluster has a refcount of exactly one, which
# is true of everything here: nothing is shared with a snapshot or a backing
# file. A reader uses it to know it may write in place; omitting it is legal
# and makes every write copy first.
OFLAG_COPIED = 1 << 63


def divide_up(value, by):
    return (value + by - 1) // by


def convert(source, destination):
    size = source.stat().st_size
    if size % CLUSTER:
        # Not a hard requirement of the format, but every disk image this
        # repository produces is a whole number of clusters, and a partial one
        # here would mean the raw image was truncated.
        sys.exit(f"mkqcow2: {source} is {size} bytes, not a multiple of {CLUSTER}")

    virtual_clusters = size // CLUSTER
    l1_size = divide_up(virtual_clusters, L2_ENTRIES)
    l1_clusters = divide_up(l1_size * 8, CLUSTER)

    # Which clusters are worth storing. A cluster of zeroes is left
    # unallocated: a reader returns zeroes for it, which is what the raw image
    # held.
    used = []
    with open(source, "rb") as raw:
        for index in range(virtual_clusters):
            chunk = raw.read(CLUSTER)
            if len(chunk) < CLUSTER:
                chunk += b"\0" * (CLUSTER - len(chunk))
            if chunk.strip(b"\0"):
                used.append(index)

    l2_needed = sorted({index // L2_ENTRIES for index in used})

    # How many refcount blocks are needed depends on how many clusters the file
    # has, which depends on how many refcount blocks there are. Two rounds
    # settle it.
    refcount_blocks = 1
    for _ in range(8):
        total = (1 + 1 + refcount_blocks + l1_clusters + len(l2_needed)
                 + len(used))
        needed = max(1, divide_up(total, REFCOUNTS_PER_BLOCK))
        if needed == refcount_blocks:
            break
        refcount_blocks = needed
    if refcount_blocks > REFCOUNT_TABLE_ENTRIES:
        sys.exit("mkqcow2: the image is too large for a one-cluster refcount table")

    # Cluster 0 is the header. Everything after it is laid out in the order a
    # reader walks it, which costs nothing and makes a hexdump legible.
    cursor = 1
    refcount_table_cluster = cursor
    cursor += 1
    refcount_block_clusters = list(range(cursor, cursor + refcount_blocks))
    cursor += refcount_blocks
    l1_cluster = cursor
    cursor += l1_clusters
    l2_clusters = {}
    for l1_index in l2_needed:
        l2_clusters[l1_index] = cursor
        cursor += 1
    data_clusters = {}
    for index in used:
        data_clusters[index] = cursor
        cursor += 1
    allocated = cursor

    l1_table = bytearray(l1_clusters * CLUSTER)
    for l1_index, cluster in l2_clusters.items():
        struct.pack_into(">Q", l1_table, l1_index * 8,
                         (cluster * CLUSTER) | OFLAG_COPIED)

    l2_tables = {l1_index: bytearray(CLUSTER) for l1_index in l2_needed}
    for index, cluster in data_clusters.items():
        table = l2_tables[index // L2_ENTRIES]
        struct.pack_into(">Q", table, (index % L2_ENTRIES) * 8,
                         (cluster * CLUSTER) | OFLAG_COPIED)

    refcounts = [bytearray(CLUSTER) for _ in range(refcount_blocks)]
    for cluster in range(allocated):
        block, offset = divmod(cluster, REFCOUNTS_PER_BLOCK)
        struct.pack_into(">H", refcounts[block], offset * 2, 1)

    refcount_table = bytearray(CLUSTER)
    for index, cluster in enumerate(refcount_block_clusters):
        struct.pack_into(">Q", refcount_table, index * 8, cluster * CLUSTER)

    header = bytearray(CLUSTER)
    struct.pack_into(
        ">4sIQIIQIIQQIIQ", header, 0,
        MAGIC, VERSION,
        0,                          # backing file offset: none
        0,                          # backing file length
        CLUSTER_BITS,
        size,                       # the virtual disk size
        0,                          # crypt_method: none
        l1_size,
        l1_cluster * CLUSTER,
        refcount_table_cluster * CLUSTER,
        1,                          # refcount table clusters
        0,                          # snapshot count
        0,                          # snapshot offset
    )
    # The three v3 feature words and the trailing two fields, which the packed
    # run above deliberately stops short of: they are not adjacent to it in any
    # natural grouping and writing them positionally makes the offsets visible.
    struct.pack_into(">QQQ", header, 0x48, 0, 0, 0)
    struct.pack_into(">II", header, 0x60, REFCOUNT_ORDER, HEADER_LENGTH)

    with open(destination, "wb") as image:
        image.truncate(allocated * CLUSTER)
        image.seek(0)
        image.write(bytes(header))
        image.seek(refcount_table_cluster * CLUSTER)
        image.write(bytes(refcount_table))
        for cluster, block in zip(refcount_block_clusters, refcounts):
            image.seek(cluster * CLUSTER)
            image.write(bytes(block))
        image.seek(l1_cluster * CLUSTER)
        image.write(bytes(l1_table))
        for l1_index, cluster in l2_clusters.items():
            image.seek(cluster * CLUSTER)
            image.write(bytes(l2_tables[l1_index]))

        with open(source, "rb") as raw:
            for index, cluster in data_clusters.items():
                raw.seek(index * CLUSTER)
                chunk = raw.read(CLUSTER)
                image.seek(cluster * CLUSTER)
                image.write(chunk.ljust(CLUSTER, b"\0"))

    return {
        "virtual": size,
        "stored": allocated * CLUSTER,
        "clusters": len(used),
        "of": virtual_clusters,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("source", help="the raw disk image to convert")
    parser.add_argument("destination", help="the qcow2 file to write")
    args = parser.parse_args()

    source = Path(args.source)
    if not source.is_file():
        sys.exit(f"mkqcow2: {source} does not exist")

    report = convert(source, args.destination)
    print(
        "mkqcow2: {virtual} bytes virtual, {stored} bytes stored, "
        "{clusters}/{of} clusters allocated".format(**report)
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
