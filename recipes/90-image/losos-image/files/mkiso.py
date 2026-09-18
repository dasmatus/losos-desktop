#!/usr/bin/env python3
"""Write a UEFI-bootable hybrid ISO, because there is no xorriso.

The result is one file that three different readers each see as something they
understand, over exactly the same bytes:

  * firmware booting from optical media follows the El Torito boot catalog to
    the ESP image and boots the UKI inside it;
  * firmware booting from a USB stick the file was written to with `dd` finds a
    GPT, an EFI system partition and a root partition, and boots the same UKI
    from the same bytes;
  * anything that mounts it sees an ISO 9660 volume with the two partition
    images in it as ordinary files.

Nothing is stored twice. The ESP volume and the root filesystem are each laid
down once, at an offset that is both an ISO 9660 extent and a megabyte-aligned
GPT partition start, and the three views are three sets of pointers at them.
That is what makes the installer's `CopyBlocks=auto` work from this medium: the
root partition it copies is a real partition on the device it booted from, not
an archive it would have to unpack.

ISO 9660 as written here is the 1988 base standard with no Rock Ridge and no
Joliet: two files in the root directory, uppercase 8.3 names with a version
suffix, and nothing that needs an extension. The files here exist to be found
by a person poking at the medium; everything that boots is addressed by the
boot catalog or by the partition table.
"""

import argparse
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import mkgpt  # noqa: E402
# The hole-preserving copy is the disk assembler's, not this file's: an ISO is
# the same assembly with a volume descriptor set in front of it.
from mkdisk import copy_into  # noqa: E402

SECTOR = 512
# ISO 9660's logical sector, which is not the disk's.
ISO_SECTOR = 2048
# The first sixteen sectors are the system area: reserved by ISO 9660, used by
# nobody, and therefore exactly where a protective MBR and a GPT fit without
# either format having to know about the other.
SYSTEM_AREA_SECTORS = 16
ALIGNMENT = 1 << 20

PRIMARY_VOLUME_DESCRIPTOR = 1
BOOT_RECORD = 0
TERMINATOR = 255

# 1980-01-01, fixed so the image is reproducible.
ISO_TIMESTAMP = b"1980010100000000" + bytes([0])
DIRECTORY_TIMESTAMP = bytes([80, 1, 1, 0, 0, 0, 0])


def both32(value):
    """ISO 9660 stores every number twice, little-endian then big-endian."""
    return struct.pack("<I", value) + struct.pack(">I", value)


def both16(value):
    return struct.pack("<H", value) + struct.pack(">H", value)


def align_up(value, to):
    return (value + to - 1) // to * to


def directory_record(name, extent, length, is_dir):
    """One directory record. `name` is bytes: b"\\x00" for ".", b"\\x01" for "..".

    The record is padded to an even length, which is why the name's length
    decides whether there is a pad byte rather than the record's.
    """
    flags = 0x02 if is_dir else 0x00
    record = (
        bytes([0, 0])
        + both32(extent)
        + both32(length)
        + DIRECTORY_TIMESTAMP
        + bytes([flags, 0, 0])
        + both16(1)
        + bytes([len(name)])
        + name
    )
    if len(record) % 2:
        record += b"\0"
    return bytes([len(record)]) + record[1:]


def path_table(root_extent, big_endian):
    """The path table, which for a volume with no subdirectories is one row."""
    pack = ">I" if big_endian else "<I"
    parent = struct.pack(">H" if big_endian else "<H", 1)
    return (
        bytes([1, 0])              # one byte of identifier, no extended attrs
        + struct.pack(pack, root_extent)
        + parent
        + b"\0"                    # the root's identifier: a single NUL
        + b"\0"                    # padded to an even length
    )


def volume_descriptor(kind, identifier=b"CD001", version=1):
    raw = bytearray(ISO_SECTOR)
    raw[0] = kind
    raw[1:6] = identifier
    raw[6] = version
    return raw


def primary_volume_descriptor(volume_id, total_sectors, path_table_l,
                              path_table_m, path_table_size, root_record):
    raw = volume_descriptor(PRIMARY_VOLUME_DESCRIPTOR)
    raw[8:40] = b" " * 32                                   # system identifier
    raw[40:72] = volume_id.upper().encode().ljust(32)[:32]
    raw[80:88] = both32(total_sectors)
    raw[120:124] = both16(1)                                # volume set size
    raw[124:128] = both16(1)                                # volume sequence
    raw[128:132] = both16(ISO_SECTOR)
    raw[132:140] = both32(path_table_size)
    raw[140:144] = struct.pack("<I", path_table_l)
    raw[148:152] = struct.pack(">I", path_table_m)
    raw[156:156 + len(root_record)] = root_record
    for start, end in ((190, 318), (318, 446), (446, 574), (574, 702)):
        raw[start:end] = b" " * (end - start)
    for start in (702, 739, 776):
        raw[start:start + 37] = b" " * 37
    raw[813:830] = ISO_TIMESTAMP
    raw[830:847] = ISO_TIMESTAMP
    # Expiration and effective dates: all-zero digits mean "not specified",
    # which is what a volume that never expires should say.
    raw[847:864] = b"0" * 16 + bytes([0])
    raw[864:881] = b"0" * 16 + bytes([0])
    raw[881] = 1                                            # structure version
    return bytes(raw)


def boot_record_descriptor(catalog_sector):
    raw = volume_descriptor(BOOT_RECORD)
    raw[7:39] = b"EL TORITO SPECIFICATION".ljust(32, b"\0")[:32]
    raw[71:75] = struct.pack("<I", catalog_sector)
    return bytes(raw)


def boot_catalog(esp_sector, esp_bytes):
    """The El Torito catalog: one validation entry and one EFI boot entry.

    Platform 0xEF is UEFI. There is deliberately no BIOS entry: this OS boots
    from a unified kernel image loaded by firmware, and nothing here could make
    a machine without UEFI boot it.
    """
    validation = bytearray(32)
    validation[0] = 1                       # header id
    validation[1] = 0xEF                    # platform: UEFI
    validation[4:28] = b"LosOS".ljust(24, b"\0")
    validation[30:32] = bytes([0x55, 0xAA])
    # The validation entry's sixteen little-endian words must sum to zero.
    total = sum(struct.unpack_from("<H", validation, i)[0] for i in range(0, 32, 2))
    struct.pack_into("<H", validation, 28, (-total) & 0xFFFF)

    sectors = esp_bytes // SECTOR
    if sectors > 0xFFFF:
        # The field is sixteen bits and a FAT32 volume cannot be smaller than
        # about 34MB, so an ESP large enough to hold a unified kernel image
        # does not fit in it. Zero is what the tools that meet the same wall
        # write, and UEFI firmware reads the boot image as a FAT volume and
        # takes its size from the BPB rather than from here. A firmware that
        # honoured this field literally would load nothing; that has to be
        # confirmed on a real machine, and docs/limits.md says so.
        sectors = 0

    entry = bytearray(32)
    entry[0] = 0x88                         # bootable
    entry[1] = 0x00                         # no emulation
    struct.pack_into("<H", entry, 2, 0)     # load segment: firmware's choice
    struct.pack_into("<H", entry, 6, sectors)
    struct.pack_into("<I", entry, 8, esp_sector)

    return bytes(validation + entry).ljust(ISO_SECTOR, b"\0")


def build(esp, root, output, volume_id, root_type, free_bytes, seed=""):
    esp_bytes = align_up(esp.stat().st_size, ALIGNMENT)
    root_bytes = align_up(root.stat().st_size, ALIGNMENT)

    # The fixed part of the volume: descriptors, boot catalog, path tables and
    # the one directory. Laid out by hand because there are seven of them and
    # naming each sector is clearer than a running counter.
    pvd_sector = SYSTEM_AREA_SECTORS
    boot_record_sector = pvd_sector + 1
    terminator_sector = pvd_sector + 2
    catalog_sector = pvd_sector + 3
    path_table_l_sector = pvd_sector + 4
    path_table_m_sector = pvd_sector + 5
    root_directory_sector = pvd_sector + 6

    # Partitions start on a megabyte, which is also an ISO sector, so the same
    # offset serves both tables.
    esp_offset = ALIGNMENT
    root_offset = esp_offset + esp_bytes
    end_of_partitions = root_offset + root_bytes

    total_bytes = end_of_partitions + align_up(free_bytes, ALIGNMENT)
    total_bytes += mkgpt.RESERVED_SECTORS * SECTOR
    total_bytes = align_up(total_bytes, ALIGNMENT)
    total_iso_sectors = total_bytes // ISO_SECTOR
    disk_sectors = total_bytes // SECTOR

    records = [
        directory_record(b"\x00", root_directory_sector, ISO_SECTOR, True),
        directory_record(b"\x01", root_directory_sector, ISO_SECTOR, True),
        directory_record(b"ESP.IMG;1", esp_offset // ISO_SECTOR, esp_bytes, False),
        directory_record(b"ROOT.IMG;1", root_offset // ISO_SECTOR, root_bytes, False),
    ]
    directory = b"".join(records)
    if len(directory) > ISO_SECTOR:
        sys.exit("mkiso: the root directory does not fit in one sector")

    root_record = directory_record(b"\x00", root_directory_sector, ISO_SECTOR, True)
    table_l = path_table(root_directory_sector, False)
    table_m = path_table(root_directory_sector, True)

    partitions = [
        mkgpt.Partition("esp", esp_offset // SECTOR, esp_bytes // SECTOR,
                        "ESP", seed=seed),
        mkgpt.Partition(root_type, root_offset // SECTOR, root_bytes // SECTOR,
                        "root", seed=seed),
    ]

    with open(output, "wb+") as image:
        image.truncate(total_bytes)

        image.seek(pvd_sector * ISO_SECTOR)
        image.write(primary_volume_descriptor(
            volume_id, total_iso_sectors, path_table_l_sector,
            path_table_m_sector, len(table_l), root_record,
        ))
        image.seek(boot_record_sector * ISO_SECTOR)
        image.write(boot_record_descriptor(catalog_sector))
        image.seek(terminator_sector * ISO_SECTOR)
        image.write(bytes(volume_descriptor(TERMINATOR)))
        image.seek(catalog_sector * ISO_SECTOR)
        image.write(boot_catalog(esp_offset // ISO_SECTOR, esp_bytes))
        image.seek(path_table_l_sector * ISO_SECTOR)
        image.write(table_l.ljust(ISO_SECTOR, b"\0"))
        image.seek(path_table_m_sector * ISO_SECTOR)
        image.write(table_m.ljust(ISO_SECTOR, b"\0"))
        image.seek(root_directory_sector * ISO_SECTOR)
        image.write(directory.ljust(ISO_SECTOR, b"\0"))

        copy_into(image, esp, esp_offset)
        copy_into(image, root, root_offset)

        # Last, because the protective MBR sits at byte zero of the system area
        # and the backup table at the very end of the file: both are outside
        # every ISO structure written above.
        report = mkgpt.write(image, disk_sectors, partitions, seed=seed)

    report["bytes"] = total_bytes
    report["iso_sectors"] = total_iso_sectors
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--esp", required=True, help="the FAT volume to boot from")
    parser.add_argument("--root", required=True, help="the root filesystem image")
    parser.add_argument("--output", required=True, help="the ISO to write")
    parser.add_argument("--volume-id", default="LOSOS", help="ISO 9660 volume identifier")
    parser.add_argument("--root-type", default="root-x86-64",
                        help="GPT type of the root partition, by its systemd name")
    parser.add_argument("--free", type=int, default=0,
                        help="free space to leave before the backup partition table")
    parser.add_argument("--seed", default="",
                        help="string the derived partition GUIDs are a function of")
    args = parser.parse_args()

    esp, root = Path(args.esp), Path(args.root)
    for path in (esp, root):
        if not path.is_file():
            sys.exit(f"mkiso: {path} does not exist")

    report = build(esp, root, args.output, args.volume_id, args.root_type,
                   args.free, args.seed)
    print(f"mkiso: {report['bytes']} bytes, {report['iso_sectors']} ISO sectors, "
          f"disk {report['disk_guid']}")
    for partition in report["partitions"]:
        print(f"mkiso:   {partition['name']}: LBA {partition['first_lba']}"
              f"-{partition['last_lba']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
