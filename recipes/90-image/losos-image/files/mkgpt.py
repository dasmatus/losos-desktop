#!/usr/bin/env python3
"""Write a GUID partition table over a set of already-placed partition images.

Used by `mkdisk.py` for the QCOW2 disk and by `mkiso.py` for the hybrid ISO,
which is the whole reason this is a module rather than part of either: the two
images differ in what surrounds the partitions, not in how the partitions are
described.

Nothing here allocates. The caller decides where each partition sits, because
on the ISO those offsets also have to satisfy ISO9660, and a partitioner that
chose for itself would have to be told about a constraint it has no business
knowing.

The partition type is named rather than spelled as a GUID, the same names
`manifest/architectures.yaml` uses, so no recipe and no caller writes an
architecture-specific constant.
"""

import binascii
import hashlib
import struct
import sys

SECTOR = 512
# A GPT reserves 128 entries of 128 bytes, which is 32 sectors, and every
# implementation assumes that layout even though the header says otherwise.
ENTRY_SIZE = 128
ENTRY_COUNT = 128
ENTRY_SECTORS = ENTRY_SIZE * ENTRY_COUNT // SECTOR
# One header sector plus the entry array, at each end of the disk.
RESERVED_SECTORS = 1 + ENTRY_SECTORS

HEADER_SIZE = 92
GPT_SIGNATURE = b"EFI PART"
GPT_REVISION = 0x00010000

# The discoverable-partition GUIDs, by the names architectures.yaml uses.
# systemd-gpt-auto-generator matches on these, which is why this OS needs no
# /etc/fstab and no root= on the kernel command line: get one wrong and the
# image builds, boots, and drops into an initrd that cannot find a root.
TYPES = {
    "esp": "C12A7328-F81F-11D2-BA4B-00A0C93EC93B",
    "root-x86-64": "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709",
    "root-x86-64-verity": "2C7357ED-EBD2-46D9-AEC1-23D437EC2BF5",
    "root-arm64": "B921B045-1DF0-41C3-AF44-4C6F280D3FAE",
    "root-arm64-verity": "DF3300CE-D69F-4C92-978C-9BFB0F38D820",
    "home": "933AC7E1-2EB4-4F13-B844-0E14E2AEF915",
    "linux-generic": "0FC63DAF-8483-4772-8E79-3D69D8477DE4",
}


def guid_bytes(text):
    """A GUID in its on-disk form.

    The first three fields are stored little-endian and the last two big-endian
    -- a mixed order inherited from Microsoft's GUID struct. Written out rather
    than hidden behind uuid.UUID.bytes_le so that the asymmetry is visible at
    the point where getting it wrong produces a partition type nothing matches.
    """
    fields = text.strip("{}").split("-")
    if len(fields) != 5:
        sys.exit(f"mkgpt: {text!r} is not a GUID")
    first, second, third, fourth, fifth = (bytes.fromhex(f) for f in fields)
    return (
        first[::-1] + second[::-1] + third[::-1] + fourth + fifth
    )


def guid_text(raw):
    return "-".join([
        raw[0:4][::-1].hex().upper(), raw[4:6][::-1].hex().upper(),
        raw[6:8][::-1].hex().upper(), raw[8:10].hex().upper(),
        raw[10:16].hex().upper(),
    ])


def derive_guid(*parts):
    """A GUID that is a function of its inputs rather than of the clock.

    A partition GUID is normally random. Deriving it keeps the image
    reproducible while still giving two different builds two different
    identifiers, which is what anything that addresses a partition by GUID
    needs of it.
    """
    digest = hashlib.sha256()
    for part in parts:
        digest.update(str(part).encode() + b"\0")
    raw = bytearray(digest.digest()[:16])
    raw[7] = (raw[7] & 0x0F) | 0x40   # version 4, in the stored byte order
    raw[8] = (raw[8] & 0x3F) | 0x80   # variant 1
    return bytes(raw)


class Partition:
    def __init__(self, kind, first_lba, sectors, name, seed=""):
        if kind not in TYPES:
            sys.exit(f"mkgpt: unknown partition type {kind!r}; "
                     f"known: {', '.join(sorted(TYPES))}")
        self.type_guid = guid_bytes(TYPES[kind])
        self.first_lba = first_lba
        self.last_lba = first_lba + sectors - 1
        self.name = name
        self.unique_guid = derive_guid("losos-desktop/gpt/1", kind, name, seed)

    def entry(self):
        return (
            self.type_guid
            + self.unique_guid
            + struct.pack("<QQQ", self.first_lba, self.last_lba, 0)
            + self.name.encode("utf-16-le").ljust(72, b"\0")[:72]
        )


def protective_mbr(disk_sectors):
    """The MBR that keeps a GPT-unaware tool from thinking the disk is empty.

    One partition of type 0xEE covering the whole disk. A tool that only reads
    MBRs sees a disk that is entirely in use by something it does not
    understand, which is the point: the alternative is one that sees free space
    and offers to partition it.
    """
    raw = bytearray(SECTOR)
    size = min(disk_sectors - 1, 0xFFFFFFFF)
    raw[446:462] = struct.pack(
        "<BBBBBBBBII",
        0x00,                    # not bootable
        0x00, 0x02, 0x00,        # starting CHS, the conventional 0/0/2
        0xEE,                    # the protective type
        0xFF, 0xFF, 0xFF,        # ending CHS, saturated
        1,                       # first LBA
        size,
    )
    struct.pack_into("<H", raw, 510, 0xAA55)
    return bytes(raw)


def header(my_lba, alternate_lba, entries_lba, disk_sectors, disk_guid,
           entries_crc, first_usable, last_usable):
    raw = bytearray(HEADER_SIZE)
    struct.pack_into(
        "<8sIIIIQQQQ", raw, 0,
        GPT_SIGNATURE, GPT_REVISION, HEADER_SIZE,
        0,                       # header CRC, filled in below
        0,                       # reserved
        my_lba, alternate_lba, first_usable, last_usable,
    )
    raw[56:72] = disk_guid
    struct.pack_into("<QIII", raw, 72, entries_lba, ENTRY_COUNT, ENTRY_SIZE,
                     entries_crc)
    # The header's own checksum is taken with the field zeroed, which is why it
    # cannot be packed in the first pass.
    struct.pack_into("<I", raw, 16, binascii.crc32(bytes(raw)) & 0xFFFFFFFF)
    return bytes(raw)


def write(image, disk_sectors, partitions, seed=""):
    """Write the protective MBR and both copies of the table into `image`.

    `image` is an open file. The partitions' contents are expected to be there
    already; this only describes them.
    """
    for partition in partitions:
        if partition.last_lba >= disk_sectors - RESERVED_SECTORS:
            sys.exit(
                f"mkgpt: partition {partition.name!r} ends at LBA "
                f"{partition.last_lba}, inside the space the backup table needs"
            )

    entries = bytearray(ENTRY_COUNT * ENTRY_SIZE)
    for index, partition in enumerate(partitions):
        entries[index * ENTRY_SIZE:(index + 1) * ENTRY_SIZE] = partition.entry()
    entries_crc = binascii.crc32(bytes(entries)) & 0xFFFFFFFF

    disk_guid = derive_guid("losos-desktop/gpt-disk/1", seed, disk_sectors)
    first_usable = 1 + ENTRY_SECTORS + 1
    last_usable = disk_sectors - RESERVED_SECTORS - 1
    backup_entries_lba = disk_sectors - 1 - ENTRY_SECTORS

    image.seek(0)
    image.write(protective_mbr(disk_sectors))
    image.seek(SECTOR)
    image.write(header(1, disk_sectors - 1, 2, disk_sectors, disk_guid,
                       entries_crc, first_usable, last_usable))
    image.seek(2 * SECTOR)
    image.write(bytes(entries))

    # The backup: the same entries, then a header that names itself last and
    # the primary first. A disk whose backup header is missing or wrong is one
    # that every tool offers to "repair", usually by guessing.
    image.seek(backup_entries_lba * SECTOR)
    image.write(bytes(entries))
    image.seek((disk_sectors - 1) * SECTOR)
    image.write(header(disk_sectors - 1, 1, backup_entries_lba, disk_sectors,
                       disk_guid, entries_crc, first_usable, last_usable))

    return {
        "disk_guid": guid_text(disk_guid),
        "partitions": [
            {
                "name": partition.name,
                "type": guid_text(partition.type_guid),
                "uuid": guid_text(partition.unique_guid),
                "first_lba": partition.first_lba,
                "last_lba": partition.last_lba,
            }
            for partition in partitions
        ],
    }
