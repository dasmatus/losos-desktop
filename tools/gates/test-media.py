#!/usr/bin/env python3
"""Prove that `assert-media.py` rejects the images that would not boot.

`recipes/90-image/losos-image/files/assert-media.py` is the build's last word
on whether the disk and the ISO came out right, and it runs as a `Test` step
inside the jail, on the real artefacts -- which means it runs exactly once,
four hours in, and only when everything before it succeeded. A checker that is
wrong in the accepting direction never says so: the build goes green and the
failure surfaces as a machine that powers on to an empty boot menu.

So the checker is tested here instead, against images this file writes from the
format. That is deliberately the same discipline `tools/gates/test-image.py`
applies to `mkcpio.py` and `mkuki.py`, with the halves swapped: there a writer
is read back by a second implementation, here a reader is fed by one.

The images below are not the images the build produces. They are the smallest
thing that has the structure `assert-media.py` has an opinion about, and then
one mutation each. What is being checked is that every mutation is caught --
an accept-everything checker passes any test made only of good images.

Nothing here touches mkosi, systemd-repart or xorriso. Whether *those* write a
correct GPT is upstream's business and not something this repository can
usefully re-derive; whether this repository correctly reads one back is.
"""

import contextlib
import importlib.util
import io
import struct
import sys
import uuid
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CHECKER = REPO / "recipes/90-image/losos-image/files/assert-media.py"

SECTOR = 512
ISO_SECTOR = 2048

# From the Discoverable Partitions Specification. `ROOT_X86_64` is what the
# image layer writes on this architecture; `LINUX_DATA` is the type a partition
# gets when nobody says otherwise, and is the near-miss that matters most --
# an image typed that way builds, mounts by hand, and never boots, because
# systemd-gpt-auto-generator finds no root and there is no root= to fall back
# on.
ESP = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B"
ROOT_X86_64 = "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709"
LINUX_DATA = "0FC63DAF-8483-4772-8E79-3D69D8477DE4"
# What xorriso types the ISO 9660 image area as when it appends the real
# partitions after it. Present here for one reason: it occupies entry 1, so a
# checker reading partitions by index looks at this and not at the ESP.
ISO_AREA = "96CD1DD7-D5A4-4A3E-8F1E-4C9EA5B3A9FD"


def load_checker():
    """Import assert-media.py, whose name is not an identifier."""
    spec = importlib.util.spec_from_file_location("assert_media", CHECKER)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def guid_bytes(text):
    """A type GUID in the mixed-endian form a partition table stores."""
    return uuid.UUID(text).bytes_le


def fat32_volume(size):
    """A region that begins with something a FAT32 boot sector looks like.

    Only the two fields `assert-media.py` reads are real. A filesystem that
    mounts is not the point: the checker's claim is "the ESP begins with a
    FAT32 volume", and this is the smallest thing about which that claim is
    true.
    """
    sector = bytearray(SECTOR)
    sector[0:3] = b"\xeb\x58\x90"
    sector[3:11] = b"MSDOS5.0"
    sector[82:90] = b"FAT32   "
    sector[510:512] = b"\x55\xaa"
    return bytes(sector) + bytes(size - SECTOR)


def ext4_volume(size):
    """A region whose superblock carries the ext4 magic, 1024 bytes in."""
    body = bytearray(size)
    struct.pack_into("<H", body, 1024 + 0x38, 0xEF53)
    return bytes(body)


def gpt(disk_size, entries):
    """A primary GPT header and table, as bytes to splice in at LBA 1.

    `entries` is a list of (type GUID text, first LBA, last LBA, name). The
    CRCs are not computed: `assert-media.py` does not check them, and a CRC
    this file computed and that file ignored would be a test of nothing. The
    tombstone matters more than the four bytes -- if the checker ever starts
    validating them, this is where the fixture has to grow.
    """
    entry_size = 128
    entries_lba = 2
    count = max(len(entries), 4)

    table = bytearray(count * entry_size)
    for index, (type_guid, first, last, name) in enumerate(entries):
        at = index * entry_size
        table[at:at + 16] = guid_bytes(type_guid)
        table[at + 16:at + 32] = uuid.uuid5(uuid.NAMESPACE_OID, name).bytes_le
        struct.pack_into("<QQQ", table, at + 32, first, last, 0)
        encoded = name.encode("utf-16-le")[:70]
        table[at + 56:at + 56 + len(encoded)] = encoded

    header = bytearray(SECTOR)
    header[0:8] = b"EFI PART"
    struct.pack_into("<IIII", header, 8, 0x00010000, 92, 0, 0)
    struct.pack_into("<QQ", header, 24, 1, disk_size // SECTOR - 1)
    struct.pack_into("<QQQ", header, 40, 34, disk_size // SECTOR - 34, 0)
    header[56:72] = uuid.uuid5(uuid.NAMESPACE_OID, "disk").bytes_le
    struct.pack_into("<QIII", header, 72, entries_lba, count, entry_size, 0)

    return bytes(header) + bytes(table)


def raw_disk(root_type=ROOT_X86_64, esp_body=None, root_body=None):
    """A GPT disk with an ESP and a root partition, in that order.

    Laid out on 1 MiB boundaries the way every partitioner has done since
    4K-native drives arrived, so the offsets look like a real disk's rather
    than like the smallest thing that fits.
    """
    mib = 1024 * 1024
    esp_at, esp_size = 1 * mib, 2 * mib
    root_at, root_size = 3 * mib, 2 * mib
    disk = bytearray(8 * mib)

    disk[esp_at:esp_at + esp_size] = (
        esp_body if esp_body is not None else fat32_volume(esp_size)
    )
    disk[root_at:root_at + root_size] = (
        root_body if root_body is not None else ext4_volume(root_size)
    )

    table = gpt(len(disk), [
        (ESP, esp_at // SECTOR, (esp_at + esp_size) // SECTOR - 1, "esp"),
        (root_type, root_at // SECTOR, (root_at + root_size) // SECTOR - 1,
         "root"),
    ])
    disk[SECTOR:SECTOR + len(table)] = table
    return bytes(disk)


def qcow2(raw, version=3, cluster_bits=16):
    """Wrap a raw disk in qcow2, allocating only the clusters that are not zero.

    Sparse on purpose. `assert-media.py` reads the partition table out of the
    qcow2 rather than out of the raw image it was converted from, and the
    reason is that a conversion which dropped every cluster would still pass a
    check against the raw file. A fixture that allocated everything would not
    exercise the L1/L2 walk that catches it.
    """
    cluster = 1 << cluster_bits
    total = (len(raw) + cluster - 1) // cluster
    l2_entries = cluster // 8
    l1_size = (total + l2_entries - 1) // l2_entries

    used = [i for i in range(total)
            if any(raw[i * cluster:(i + 1) * cluster])]

    # cluster 0 is the header, then the L1 table, then one L2 table per L1
    # entry, then the data.
    l1_at = 1 * cluster
    l2_at = [(1 + 1 + i) * cluster for i in range(l1_size)]
    data_at = {c: (1 + 1 + l1_size + n) * cluster
               for n, c in enumerate(used)}

    size = (1 + 1 + l1_size + len(used)) * cluster
    image = bytearray(size)

    image[0:4] = b"QFI\xfb"
    struct.pack_into(">I", image, 4, version)
    struct.pack_into(">I", image, 20, cluster_bits)
    struct.pack_into(">Q", image, 24, len(raw))
    struct.pack_into(">I", image, 36, l1_size)
    struct.pack_into(">Q", image, 40, l1_at)
    struct.pack_into(">I", image, 48, 1)          # one refcount table cluster
    if version >= 3:
        struct.pack_into(">I", image, 100, 0)     # incompatible features
        struct.pack_into(">I", image, 104, 72)    # header length

    for index, at in enumerate(l2_at):
        # Bit 63 marks a cluster that need not be copied on write. The reader
        # masks it off; it is set because a real qcow2 sets it, and a fixture
        # that left it clear would not prove the mask works.
        struct.pack_into(">Q", image, l1_at + index * 8, at | (1 << 63))

    for guest, host in data_at.items():
        l1_index, l2_index = divmod(guest, l2_entries)
        struct.pack_into(">Q", image, l2_at[l1_index] + l2_index * 8,
                         host | (1 << 63))
        image[host:host + cluster] = raw[guest * cluster:
                                         (guest + 1) * cluster
                                         ].ljust(cluster, b"\0")

    return bytes(image)


def hybrid_iso(root_type=ROOT_X86_64, catalog_platform=0xEF,
               bootable=0x88, boot_extent_shift=0, with_gpt=True):
    """An ISO 9660 image with El Torito and a GPT over the same ESP.

    The structure the image layer's `xorriso` line produces: the ISO's own
    image area is partition 1, and the real partitions are appended after it,
    which is exactly why `assert-media.py` looks partitions up by type. If it
    went by index it would read the ISO area here and report on the wrong
    bytes -- so the fixture keeps that first entry even though nothing reads
    it.
    """
    mib = 1024 * 1024
    esp_at, esp_size = 2 * mib, 2 * mib
    root_at, root_size = 4 * mib, 2 * mib
    image = bytearray(8 * mib)

    # Primary volume descriptor.
    image[16 * ISO_SECTOR] = 1
    image[16 * ISO_SECTOR + 1:16 * ISO_SECTOR + 6] = b"CD001"
    image[16 * ISO_SECTOR + 6] = 1

    # Boot record volume descriptor, pointing at the catalog.
    catalog_sector = 19
    boot = 17 * ISO_SECTOR
    image[boot] = 0
    image[boot + 1:boot + 6] = b"CD001"
    image[boot + 6] = 1
    image[boot + 7:boot + 30] = b"EL TORITO SPECIFICATION"
    struct.pack_into("<I", image, boot + 71, catalog_sector)

    # Terminator, so the descriptor sequence is well formed even though
    # nothing here reads it.
    image[18 * ISO_SECTOR] = 0xFF
    image[18 * ISO_SECTOR + 1:18 * ISO_SECTOR + 6] = b"CD001"

    # Boot catalog: a validation entry, then one default entry for UEFI.
    catalog = catalog_sector * ISO_SECTOR
    image[catalog] = 1
    image[catalog + 1] = catalog_platform
    image[catalog + 30:catalog + 32] = b"\x55\xaa"
    image[catalog + 32] = bootable
    struct.pack_into("<I", image, catalog + 40,
                     esp_at // ISO_SECTOR + boot_extent_shift)

    image[esp_at:esp_at + esp_size] = fat32_volume(esp_size)
    image[root_at:root_at + root_size] = ext4_volume(root_size)

    if with_gpt:
        table = gpt(len(image), [
            (ISO_AREA, 0, esp_at // SECTOR - 1, "iso"),
            (ESP, esp_at // SECTOR, (esp_at + esp_size) // SECTOR - 1, "esp"),
            (root_type, root_at // SECTOR,
             (root_at + root_size) // SECTOR - 1, "root"),
        ])
        image[SECTOR:SECTOR + len(table)] = table

    return bytes(image)


def run(checker, work, qcow2_bytes, iso_bytes):
    """Run both checks over a pair of images, quietly, and report the verdict."""
    disk = work / "losos.qcow2"
    medium = work / "losos.iso"
    disk.write_bytes(qcow2_bytes)
    medium.write_bytes(iso_bytes)

    noise = io.StringIO()
    with contextlib.redirect_stdout(noise), contextlib.redirect_stderr(noise):
        ok = checker.check_qcow2(disk)
        ok = checker.check_iso(medium) and ok
    return ok, noise.getvalue().strip()


def main():
    checker = load_checker()
    work = Path(__file__).resolve().parent.parent.parent / "out" / "test-media"
    work.mkdir(parents=True, exist_ok=True)

    good_disk = raw_disk()
    good_iso = hybrid_iso()

    # The fixture's own precondition, not a case. The ISO's first partition
    # entry is the ISO 9660 image area, so looking partitions up by index
    # reads that and not the ESP -- which is the bug `by_type` exists to
    # prevent and the reason every case below means anything. If a future
    # edit drops that first entry, the cases would all still pass while
    # testing nothing, so this says so instead.
    table = checker.partitions(io.BytesIO(good_iso))
    if checker.guid_text(table[0][0]) == ESP:
        print("test-media: the ISO fixture no longer puts the image area "
              "first, so nothing here exercises by_type", file=sys.stderr)
        return 1

    # Each case is (name, qcow2, iso, should_pass, why it is here).
    cases = [
        ("the shape the image layer produces",
         qcow2(good_disk), good_iso, True,
         "an ESP and a discoverable root, El Torito over the same bytes"),

        ("a root typed as Linux data is rejected",
         qcow2(raw_disk(root_type=LINUX_DATA)), good_iso, False,
         "gpt-auto finds no root and there is no root= to fall back on"),

        ("an ISO whose root is typed as Linux data is rejected",
         qcow2(good_disk), hybrid_iso(root_type=LINUX_DATA), False,
         "the installer boots and then has nothing to copy"),

        ("an ESP that is not FAT32 is rejected",
         qcow2(raw_disk(esp_body=bytes(2 * 1024 * 1024))), good_iso, False,
         "firmware mounts nothing and falls through to the next device"),

        ("a root partition with no ext4 superblock is rejected",
         qcow2(raw_disk(root_body=bytes(2 * 1024 * 1024))), good_iso, False,
         "the partition is typed right and holds nothing"),

        ("a qcow2 that is version 2 is rejected",
         qcow2(good_disk, version=2), good_iso, False,
         "v2 has no way to say which features an image needs"),

        ("an El Torito entry that misses the ESP is rejected",
         qcow2(good_disk), hybrid_iso(boot_extent_shift=1), False,
         "the DVD boots one image and the USB stick another"),

        ("a boot catalog for BIOS rather than UEFI is rejected",
         qcow2(good_disk), hybrid_iso(catalog_platform=0x00), False,
         "nothing in this distribution can boot on CSM"),

        ("a boot entry not marked bootable is rejected",
         qcow2(good_disk), hybrid_iso(bootable=0x00), False,
         "firmware skips the entry and the medium looks empty"),

        ("an ISO with no GPT is rejected",
         qcow2(good_disk), hybrid_iso(with_gpt=False), False,
         "it would boot as a DVD and not as a USB stick"),
    ]

    failures = 0
    for name, disk_bytes, iso_bytes, should_pass, why in cases:
        got, noise = run(checker, work, disk_bytes, iso_bytes)
        if got == should_pass:
            print(f"  ok      {name:<50} ({why})")
            continue
        failures += 1
        verdict = "accepted" if got else "rejected"
        print(f"  FAIL    {name:<50} (assert-media {verdict} it)")
        if noise:
            for line in noise.splitlines():
                print(f"            {line}")

    if failures:
        print(f"test-media: {failures} case(s) behaved wrongly", file=sys.stderr)
        return 1

    rejected = sum(1 for case in cases if not case[3])
    print(f"test-media: assert-media accepts the real shape and rejects "
          f"{rejected} near-miss(es)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
