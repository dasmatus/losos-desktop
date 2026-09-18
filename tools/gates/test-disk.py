#!/usr/bin/env python3
"""Exercise the disk-image writers against independently written readers.

`mkext4.py`, `mkfat.py`, `mkgpt.py`, `mkiso.py` and `mkqcow2.py` exist because
no program pm will run can produce what they produce: `mkfs`, `losetup`,
`mount`, `xorriso` and `qemu-img` are outside pm's fingerprint table and mostly
outside the jail's privileges as well (C2, C3, C7). That makes them, along with
`mkcpio.py` and `mkuki.py`, the only code in this repository nobody upstream has
already debugged -- and unlike a failing compile, a mistake in any of them
produces an image that builds cleanly and does not boot.

So each one is checked here by parsing its output back with a reader written
from the format's own layout rather than from the writer's helpers. A shared
misunderstanding of a field would round-trip perfectly through the writer's own
code; it does not round-trip through a second implementation.

Where a filesystem checker happens to be installed, it is run as well and its
verdict is reported. That is a bonus and never a requirement: this gate has to
pass on a machine with nothing on it but Python, which is where `./do check`
runs.
"""

import binascii
import os
import shutil
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
IMAGE = REPO / "recipes" / "90-image" / "losos-image" / "files"
sys.path.insert(0, str(IMAGE))

import mkdisk  # noqa: E402
import mkext4  # noqa: E402
import mkfat  # noqa: E402
import mkgpt  # noqa: E402
import mkiso  # noqa: E402
import mkqcow2  # noqa: E402

SECTOR = 512


# --------------------------------------------------------------- ext4 reader

class Ext4Reader:
    """An ext4 reader written against the on-disk layout, not against mkext4."""

    def __init__(self, path):
        self.image = open(path, "rb")
        superblock = self.at(1024, 1024)
        magic = struct.unpack_from("<H", superblock, 0x38)[0]
        assert magic == 0xEF53, f"superblock magic is {magic:#x}"
        self.inodes_count = struct.unpack_from("<I", superblock, 0)[0]
        self.blocks_count = struct.unpack_from("<I", superblock, 4)[0]
        self.free_blocks = struct.unpack_from("<I", superblock, 0x0C)[0]
        self.free_inodes = struct.unpack_from("<I", superblock, 0x10)[0]
        self.first_data_block = struct.unpack_from("<I", superblock, 0x14)[0]
        self.block_size = 1024 << struct.unpack_from("<I", superblock, 0x18)[0]
        self.blocks_per_group = struct.unpack_from("<I", superblock, 0x20)[0]
        self.inodes_per_group = struct.unpack_from("<I", superblock, 0x28)[0]
        self.inode_size = struct.unpack_from("<H", superblock, 0x58)[0]
        self.compat, self.incompat, self.ro_compat = struct.unpack_from(
            "<III", superblock, 0x5C
        )
        self.reserved_gdt = struct.unpack_from("<H", superblock, 0xCE)[0]
        self.journal_inode = struct.unpack_from("<I", superblock, 0xE0)[0]
        self.groups = -(-self.blocks_count // self.blocks_per_group)

    def at(self, offset, length):
        self.image.seek(offset)
        return self.image.read(length)

    def block(self, number):
        return self.at(number * self.block_size, self.block_size)

    def descriptor(self, group):
        table = (self.first_data_block + 1) * self.block_size
        return self.at(table + group * 32, 32)

    def inode(self, number):
        group, index = divmod(number - 1, self.inodes_per_group)
        table = struct.unpack_from("<I", self.descriptor(group), 8)[0]
        return self.at(table * self.block_size + index * self.inode_size,
                       self.inode_size)

    def extents(self, raw):
        """Every (logical, length, physical) run of a file, walking the tree."""
        def walk(body):
            magic, entries, _, depth, _ = struct.unpack_from("<HHHHI", body, 0)
            assert magic == 0xF30A, f"extent magic is {magic:#x}"
            found = []
            for index in range(entries):
                offset = 12 + index * 12
                if depth == 0:
                    logical, length, high, low = struct.unpack_from(
                        "<IHHI", body, offset
                    )
                    found.append((logical, length, (high << 32) | low))
                else:
                    _, low, high, _ = struct.unpack_from("<IIHH", body, offset)
                    found += walk(self.block((high << 32) | low))
            return found

        return walk(raw[0x28:0x28 + 60])

    def contents(self, number):
        raw = self.inode(number)
        size = (struct.unpack_from("<I", raw, 4)[0]
                | (struct.unpack_from("<I", raw, 0x6C)[0] << 32))
        mode = struct.unpack_from("<H", raw, 0)[0]
        if mode & 0xF000 == 0xA000 and size < 60:
            return raw[0x28:0x28 + size]
        data = bytearray(size)
        for logical, length, physical in self.extents(raw):
            chunk = self.at(physical * self.block_size, length * self.block_size)
            start = logical * self.block_size
            data[start:start + len(chunk)] = chunk[:max(0, size - start)]
        return bytes(data[:size])

    def listdir(self, number):
        raw = self.inode(number)
        entries = {}
        for logical, length, physical in self.extents(raw):
            for index in range(length):
                block = self.block(physical + index)
                offset = 0
                while offset < len(block):
                    ino, rec_len, name_len, file_type = struct.unpack_from(
                        "<IHBB", block, offset
                    )
                    if rec_len == 0:
                        break
                    name = block[offset + 8:offset + 8 + name_len].decode()
                    if ino and name not in (".", ".."):
                        entries[name] = (ino, file_type)
                    offset += rec_len
        return entries

    def walk(self, number=2, prefix=""):
        found = {}
        for name, (ino, file_type) in self.listdir(number).items():
            path = f"{prefix}/{name}"
            if file_type == 2:
                found.update(self.walk(ino, path))
            else:
                found[path] = (file_type, self.contents(ino))
        return found

    def xattrs(self, number):
        """Attributes from inside the inode and from an attribute block."""
        raw = self.inode(number)
        found = {}
        extra = struct.unpack_from("<H", raw, 0x80)[0]
        start = 128 + extra
        if start < len(raw) and struct.unpack_from("<I", raw, start)[0] == 0xEA020000:
            found.update(self.parse_xattrs(raw, start + 4, start + 4, len(raw)))
        block = struct.unpack_from("<I", raw, 0x68)[0]
        if block:
            body = self.block(block)
            assert struct.unpack_from("<I", body, 0)[0] == 0xEA020000
            found.update(self.parse_xattrs(body, 32, 32, len(body)))
        return found

    @staticmethod
    def parse_xattrs(body, offset, value_base, limit):
        prefixes = {index: name for index, name in mkext4.XATTR_PREFIXES}
        found = {}
        while offset + 4 <= limit:
            name_len, name_index = body[offset], body[offset + 1]
            if name_len == 0:
                break
            value_offset, _, value_size = struct.unpack_from("<HII", body, offset + 2)
            name = body[offset + 16:offset + 16 + name_len].decode()
            prefix = prefixes[name_index]
            full = prefix + name if prefix.endswith(".") else prefix
            start = value_base + value_offset
            found[full] = body[start:start + value_size]
            offset += (16 + name_len + 3) // 4 * 4
        return found


# ---------------------------------------------------------------- FAT reader

class FatReader:
    """A FAT32 reader written against the format, not against mkfat."""

    def __init__(self, path):
        self.data = Path(path).read_bytes()
        assert self.data[510:512] == b"\x55\xaa", "no boot signature"
        assert self.data[82:90] == b"FAT32   ", self.data[82:90]
        self.bytes_per_sector, self.sectors_per_cluster, self.reserved = \
            struct.unpack_from("<HBH", self.data, 11)
        self.fats = self.data[16]
        assert struct.unpack_from("<H", self.data, 17)[0] == 0, \
            "a FAT32 volume declares no fixed root directory"
        assert struct.unpack_from("<H", self.data, 22)[0] == 0, \
            "a FAT32 volume declares no 16-bit FAT size"
        self.total_sectors = struct.unpack_from("<I", self.data, 32)[0]
        self.fat_sectors = struct.unpack_from("<I", self.data, 36)[0]
        self.root_cluster = struct.unpack_from("<I", self.data, 44)[0]
        self.fat_offset = self.reserved * self.bytes_per_sector
        self.data_offset = (
            (self.reserved + self.fats * self.fat_sectors) * self.bytes_per_sector
        )
        span = self.fat_sectors * self.bytes_per_sector
        assert self.data[self.fat_offset:self.fat_offset + span] == \
            self.data[self.fat_offset + span:self.fat_offset + 2 * span], \
            "the two copies of the FAT differ"
        assert self.data[510:512] == self.data[6 * self.bytes_per_sector + 510:
                                               6 * self.bytes_per_sector + 512], \
            "the backup boot sector is missing"

    def chain(self, cluster):
        found = []
        while 2 <= cluster < 0x0FFFFFF8:
            found.append(cluster)
            cluster = struct.unpack_from(
                "<I", self.data, self.fat_offset + cluster * 4
            )[0] & 0x0FFFFFFF
            assert len(found) < 1 << 22, "the cluster chain loops"
        return found

    def read(self, cluster):
        span = self.sectors_per_cluster * self.bytes_per_sector
        out = bytearray()
        for number in self.chain(cluster):
            start = self.data_offset + (number - 2) * span
            out += self.data[start:start + span]
        return bytes(out)

    def listdir(self, cluster):
        raw = self.read(cluster)
        found, long_parts = [], []
        for offset in range(0, len(raw), 32):
            entry = raw[offset:offset + 32]
            if entry[0] == 0:
                break
            if entry[0] == 0xE5:
                continue
            attributes = entry[11]
            if attributes == 0x0F:
                long_parts.append((entry[0] & 0x3F,
                                   entry[1:11] + entry[14:26] + entry[28:32]))
                continue
            if attributes & 0x08:       # the volume label
                long_parts = []
                continue
            stem = entry[0:8].decode("ascii").rstrip()
            suffix = entry[8:11].decode("ascii").rstrip()
            name = stem + ("." + suffix if suffix else "")
            if long_parts:
                long_parts.sort()
                text = b"".join(part for _, part in long_parts).decode("utf-16-le")
                name = text.split("\0")[0]
            long_parts = []
            first = ((struct.unpack_from("<H", entry, 20)[0] << 16)
                     | struct.unpack_from("<H", entry, 26)[0])
            size = struct.unpack_from("<I", entry, 28)[0]
            found.append((name, bool(attributes & 0x10), first, size))
        return found

    def walk(self, cluster=None, prefix=""):
        found = {}
        for name, is_dir, first, size in self.listdir(cluster or self.root_cluster):
            if name in (".", ".."):
                continue
            path = f"{prefix}/{name}"
            if is_dir:
                found.update(self.walk(first, path))
            else:
                found[path] = self.read(first)[:size] if first else b""
        return found


# ---------------------------------------------------------------- GPT reader

def read_gpt(path):
    """Both copies of the partition table, with every checksum verified."""
    data = Path(path).read_bytes()
    assert data[450] == 0xEE, "the protective MBR does not cover the disk"
    assert data[510:512] == b"\x55\xaa", "the protective MBR has no signature"

    def header(offset):
        raw = data[offset:offset + 92]
        assert raw[0:8] == b"EFI PART", "not a GPT header"
        stored = struct.unpack_from("<I", raw, 16)[0]
        blanked = raw[:16] + b"\0" * 4 + raw[20:]
        assert binascii.crc32(blanked) & 0xFFFFFFFF == stored, \
            "the GPT header checksum is wrong"
        my_lba, alternate, first_usable, last_usable = struct.unpack_from(
            "<QQQQ", raw, 24
        )
        entries_lba, count, size, entries_crc = struct.unpack_from("<QIII", raw, 72)
        body = data[entries_lba * SECTOR:entries_lba * SECTOR + count * size]
        assert binascii.crc32(body) & 0xFFFFFFFF == entries_crc, \
            "the GPT entry array checksum is wrong"
        partitions = []
        for index in range(count):
            entry = body[index * size:(index + 1) * size]
            if entry[0:16] == bytes(16):
                continue
            first, last, _ = struct.unpack_from("<QQQ", entry, 32)
            partitions.append({
                "type": mkgpt.guid_text(entry[0:16]),
                "uuid": mkgpt.guid_text(entry[16:32]),
                "first_lba": first,
                "last_lba": last,
                "name": entry[56:128].decode("utf-16-le").rstrip("\0"),
            })
        return {
            "my_lba": my_lba, "alternate_lba": alternate,
            "first_usable": first_usable, "last_usable": last_usable,
            "partitions": partitions,
        }

    primary = header(SECTOR)
    backup = header(primary["alternate_lba"] * SECTOR)
    assert primary["partitions"] == backup["partitions"], \
        "the primary and backup partition tables disagree"
    assert backup["alternate_lba"] == 1, "the backup header does not point back"
    assert primary["alternate_lba"] * SECTOR == len(data) - SECTOR, \
        "the backup header is not in the last sector"
    return primary


# ------------------------------------------------------------- qcow2 reader

def read_qcow2(path):
    """Reconstruct the raw image a qcow2 file stands for."""
    data = Path(path).read_bytes()
    (magic, version, _, _, cluster_bits, size, crypt, l1_size, l1_offset,
     refcount_offset, refcount_clusters, snapshots, _) = struct.unpack_from(
        ">4sIQIIQIIQQIIQ", data, 0
    )
    assert magic == b"QFI\xfb", "not a qcow2 file"
    assert version == 3, f"qcow2 version {version}"
    assert crypt == 0 and snapshots == 0, "unexpected encryption or snapshots"
    refcount_order = struct.unpack_from(">I", data, 0x60)[0]
    assert refcount_order == 4, f"refcount order {refcount_order}"

    cluster = 1 << cluster_bits
    l2_entries = cluster // 8
    raw = bytearray(size)
    allocated = set()
    for l1_index in range(l1_size):
        entry = struct.unpack_from(">Q", data, l1_offset + l1_index * 8)[0]
        l2_offset = entry & 0x00FFFFFFFFFFFE00
        if not l2_offset:
            continue
        allocated.add(l2_offset // cluster)
        for l2_index in range(l2_entries):
            found = struct.unpack_from(">Q", data, l2_offset + l2_index * 8)[0]
            offset = found & 0x00FFFFFFFFFFFE00
            if not offset:
                continue
            allocated.add(offset // cluster)
            start = (l1_index * l2_entries + l2_index) * cluster
            raw[start:start + cluster] = data[offset:offset + cluster]

    # Every cluster the tables point at must have a refcount of one, or a
    # writer would be free to reuse it.
    refcounts_per_block = cluster * 8 // (1 << refcount_order)
    for number in sorted(allocated):
        block_index, offset = divmod(number, refcounts_per_block)
        assert block_index < refcount_clusters * (cluster // 8), "refcount table too small"
        block = struct.unpack_from(">Q", data, refcount_offset + block_index * 8)[0]
        count = struct.unpack_from(">H", data, block + offset * 2)[0]
        assert count == 1, f"cluster {number} has refcount {count}"

    return bytes(raw)


# --------------------------------------------------------------- ISO reader

def read_iso(path):
    """The volume descriptors, the boot catalog and the root directory."""
    data = Path(path).read_bytes()
    sector = mkiso.ISO_SECTOR

    primary = data[16 * sector:17 * sector]
    assert primary[0] == 1 and primary[1:6] == b"CD001", "no primary volume descriptor"
    volume_id = primary[40:72].decode().rstrip()
    little, big = struct.unpack_from("<II", primary, 80)
    assert little == struct.unpack_from(">I", primary, 84)[0], \
        "the volume size disagrees with itself between byte orders"
    assert little * sector == len(data), \
        f"the volume claims {little} sectors and the file holds {len(data) // sector}"
    assert struct.unpack_from("<H", primary, 128)[0] == sector, "wrong logical block size"

    boot = data[17 * sector:18 * sector]
    assert boot[0] == 0 and boot[1:6] == b"CD001", "no boot record descriptor"
    assert boot[7:30] == b"EL TORITO SPECIFICATION", boot[7:30]
    catalog_sector = struct.unpack_from("<I", boot, 71)[0]

    terminator = data[18 * sector:19 * sector]
    assert terminator[0] == 255 and terminator[1:6] == b"CD001", "no terminator"

    catalog = data[catalog_sector * sector:(catalog_sector + 1) * sector]
    assert catalog[0] == 1, "the boot catalog has no validation entry"
    assert catalog[1] == 0xEF, f"boot platform {catalog[1]:#x} is not UEFI"
    assert catalog[30:32] == bytes([0x55, 0xAA]), "bad validation entry signature"
    total = sum(struct.unpack_from("<H", catalog, i)[0] for i in range(0, 32, 2))
    assert total & 0xFFFF == 0, "the validation entry checksum does not cancel"
    assert catalog[32] == 0x88, "the boot entry is not marked bootable"
    assert catalog[33] == 0, "the boot entry is not no-emulation"
    boot_extent = struct.unpack_from("<I", catalog, 40)[0]

    root = primary[156:190]
    root_extent = struct.unpack_from("<I", root, 2)[0]
    directory = data[root_extent * sector:(root_extent + 1) * sector]
    entries = {}
    offset = 0
    while offset < len(directory) and directory[offset]:
        length = directory[offset]
        extent = struct.unpack_from("<I", directory, offset + 2)[0]
        size = struct.unpack_from("<I", directory, offset + 10)[0]
        name_len = directory[offset + 32]
        name = directory[offset + 33:offset + 33 + name_len]
        if name not in (b"\x00", b"\x01"):
            entries[name.decode()] = (extent, size)
        offset += length

    return {
        "volume_id": volume_id,
        "boot_extent": boot_extent,
        "entries": entries,
        "sectors": little,
    }


# ------------------------------------------------------------------- fixture

def make_tree(root):
    """A tree with one of everything the writers have to represent."""
    (root / "usr" / "lib").mkdir(parents=True)
    (root / "usr" / "lib" / "os-release").write_bytes(b"ID=losos-desktop\n")
    (root / "usr" / "lib" / "blob.bin").write_bytes(bytes(range(256)) * 900)
    (root / "etc").mkdir()
    (root / "etc" / "hosts").write_bytes(b"127.0.0.1 localhost\n")
    os.link(root / "etc" / "hosts", root / "etc" / "hosts.alias")
    (root / "etc" / "short").symlink_to("hosts")
    (root / "etc" / "long").symlink_to("/" + "deep/" * 20 + "target")
    (root / "empty").mkdir()
    # Enough entries to need more than one directory block, which is where an
    # entry that straddled a block boundary would show up.
    (root / "many").mkdir()
    for index in range(200):
        (root / "many" / f"entry-{index:04d}.conf").write_text(f"{index}\n")
    try:
        os.setxattr(root / "etc" / "hosts", "user.inline", b"small")
        os.setxattr(root / "etc", "user.spilled", b"x" * 300)
    except OSError:
        # A build host whose filesystem has no attributes is not a failure;
        # the xattr checks below notice and say they were skipped.
        pass


def source_files(root):
    found = {}
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            found["/" + str(path.relative_to(root))] = (7, os.readlink(path).encode())
        elif path.is_file():
            found["/" + str(path.relative_to(root))] = (1, path.read_bytes())
    return found


def fsck(path, failures):
    """Run e2fsck if the host has one. Its absence is not a failure."""
    checker = shutil.which("e2fsck")
    if not checker:
        print("test-disk: e2fsck is not installed; "
              "the ext4 image was checked only by the reader above")
        return
    result = subprocess.run([checker, "-fn", str(path)],
                            capture_output=True, text=True)
    if result.returncode != 0:
        failures.append(
            f"e2fsck rejected the image (exit {result.returncode}):\n"
            + result.stdout.strip()
        )
    else:
        print("test-disk: e2fsck agrees the ext4 image is clean")


# --------------------------------------------------------------------- tests

def test_ext4(work, failures):
    tree = work / "tree"
    tree.mkdir()
    make_tree(tree)
    image = work / "root.raw"
    report = mkext4.Ext4Builder(tree, label="losos-root").build(image)

    reader = Ext4Reader(image)
    if reader.block_size != 4096 or reader.inode_size != 256:
        failures.append("ext4: unexpected block or inode size")
    if not reader.incompat & 0x40:
        failures.append("ext4: the extents feature is not set")
    if not reader.compat & 0x04:
        failures.append("ext4: there is no journal")

    found = reader.walk()
    expected = source_files(tree)
    if set(found) != set(expected):
        missing = sorted(set(expected) - set(found))
        extra = sorted(set(found) - set(expected))
        failures.append(f"ext4: missing {missing}, unexpected {extra}")
    else:
        for name, (kind, contents) in expected.items():
            if found[name][1] != contents:
                failures.append(f"ext4: {name} reads back differently")
            if found[name][0] != kind:
                failures.append(f"ext4: {name} has file type {found[name][0]}")

    # A hard link has to be one inode under two names, not two copies.
    entries = reader.listdir(reader.listdir(2)["etc"][0])
    if entries["hosts"][0] != entries["hosts.alias"][0]:
        failures.append("ext4: a hard link produced two inodes")

    attributes = reader.xattrs(entries["hosts"][0])
    if attributes:
        if attributes.get("user.inline") != b"small":
            failures.append(f"ext4: inline xattr reads back as {attributes}")
        spilled = reader.xattrs(reader.listdir(2)["etc"][0])
        if spilled.get("user.spilled") != b"x" * 300:
            failures.append("ext4: the attribute block reads back wrong")
    else:
        print("test-disk: the build host's filesystem has no extended "
              "attributes; the xattr paths were not exercised")

    fsck(image, failures)
    print(f"test-disk: ext4 {report['bytes']} bytes, {len(expected)} entries verified")
    return image, tree


def test_ext4_resize_inode(work, failures):
    """A filesystem big enough to need spare descriptor blocks.

    The small image above reserves none -- one descriptor block already
    addresses sixteen gigabytes -- so the resize inode, which is what lets
    systemd-repart grow the root filesystem past that, is only reachable here.
    """
    tree = work / "small"
    tree.mkdir()
    (tree / "file").write_text("x")
    image = work / "big.raw"
    mkext4.Ext4Builder(tree, label="big", size=20 << 30).build(image)

    reader = Ext4Reader(image)
    if reader.reserved_gdt == 0:
        failures.append("ext4: a 20GiB filesystem reserved no descriptor blocks")
        return
    if not reader.compat & 0x10:
        failures.append("ext4: reserved descriptor blocks without the resize inode")

    raw = reader.inode(7)
    for index in range(15):
        value = struct.unpack_from("<I", raw, 0x28 + index * 4)[0]
        if index != 13 and value:
            failures.append(f"ext4: the resize inode uses i_block[{index}]")
    dind = struct.unpack_from("<I", raw, 0x28 + 13 * 4)[0]
    if not dind:
        failures.append("ext4: the resize inode has no double-indirect block")
        return

    # Descriptor block N is slot N-1, and each one lists its own backups.
    pointers = reader.block(dind)
    gdt_blocks = -(-reader.groups * 32 // reader.block_size)
    for index in range(reader.reserved_gdt):
        block = 1 + gdt_blocks + index
        stored = struct.unpack_from("<I", pointers, (block - 1) * 4)[0]
        if stored != block:
            failures.append(
                f"ext4: resize slot {block - 1} holds {stored}, expected {block}"
            )
            break
    else:
        backups = reader.block(1 + gdt_blocks)
        expected = [
            group * reader.blocks_per_group + 1 + gdt_blocks
            for group in range(1, reader.groups)
            if mkext4.has_super(group)
        ]
        stored = [struct.unpack_from("<I", backups, i * 4)[0]
                  for i in range(len(expected))]
        if stored != expected:
            failures.append(f"ext4: descriptor backups are {stored}, expected {expected}")

    fsck(image, failures)
    print(f"test-disk: ext4 resize inode verified over "
          f"{reader.reserved_gdt} reserved descriptor blocks")


def test_ext4_extent_index(work, failures):
    """A file with more extents than fit in an inode.

    Reached by capping the extent length rather than by writing half a
    gigabyte: the code path is the same and the test stays runnable anywhere.
    """
    tree = work / "fragmented"
    tree.mkdir()
    (tree / "big.bin").write_bytes(os.urandom(4096 * 2600))
    image = work / "fragmented.raw"

    original = mkext4.MAX_EXTENT_LEN
    mkext4.MAX_EXTENT_LEN = 512
    try:
        mkext4.Ext4Builder(tree, label="fragmented").build(image)
    finally:
        mkext4.MAX_EXTENT_LEN = original

    reader = Ext4Reader(image)
    number = reader.listdir(2)["big.bin"][0]
    depth = struct.unpack_from("<H", reader.inode(number), 0x28 + 6)[0]
    if depth != 1:
        failures.append(f"ext4: expected an extent tree of depth 1, got {depth}")
    if reader.contents(number) != (tree / "big.bin").read_bytes():
        failures.append("ext4: a file behind an extent index block reads back wrong")
    fsck(image, failures)
    print("test-disk: ext4 extent index block verified")


def test_fat(work, failures):
    tree = work / "esp"
    (tree / "EFI" / "BOOT").mkdir(parents=True)
    (tree / "EFI" / "Linux").mkdir(parents=True)
    (tree / "EFI" / "BOOT" / "BOOTX64.EFI").write_bytes(b"stub" * 1000)
    # The name the OS actually ships, which is what makes long names load
    # bearing rather than decorative.
    (tree / "EFI" / "Linux" / "losos-desktop_20260918.7_x86_64.efi").write_bytes(
        os.urandom(300000)
    )
    (tree / "README.TXT").write_bytes(b"short and 8.3\n")

    image = work / "esp.img"
    report = mkfat.FatBuilder(tree, label="LOSOS_ESP").build(image)

    reader = FatReader(image)
    found = reader.walk()
    expected = {
        "/" + str(path.relative_to(tree)): path.read_bytes()
        for path in sorted(tree.rglob("*")) if path.is_file()
    }
    if set(found) != set(expected):
        failures.append(
            f"fat: names differ -- on the volume {sorted(found)}, "
            f"in the tree {sorted(expected)}"
        )
    else:
        for name, contents in expected.items():
            if found[name] != contents:
                failures.append(f"fat: {name} reads back differently")

    if reader.total_sectors * reader.bytes_per_sector != report["bytes"]:
        failures.append("fat: the boot sector's size disagrees with the file's")

    print(f"test-disk: FAT32 {report['bytes']} bytes, "
          f"{len(expected)} files verified, long names intact")
    return image


def test_gpt_and_qcow2(work, root_image, esp_image, failures):
    disk = work / "disk.raw"
    report = mkdisk.build(esp_image, root_image, disk, "root-x86-64",
                          free_bytes=64 << 20, seed="test")

    table = read_gpt(disk)
    kinds = [partition["type"] for partition in table["partitions"]]
    expected = [mkgpt.TYPES["esp"], mkgpt.TYPES["root-x86-64"]]
    if kinds != expected:
        failures.append(f"gpt: partition types are {kinds}, expected {expected}")
    for stored, described in zip(table["partitions"], report["partitions"]):
        if stored["first_lba"] != described["first_lba"]:
            failures.append(f"gpt: {stored['name']} is not where the report says")

    # The partitions must actually contain the images they name.
    with open(disk, "rb") as image:
        image.seek(table["partitions"][1]["first_lba"] * SECTOR)
        head = image.read(2048)
    if head[1024 + 0x38:1024 + 0x3A] != struct.pack("<H", 0xEF53):
        failures.append("gpt: the root partition does not start with an ext4 superblock")

    qcow2 = work / "disk.qcow2"
    mkqcow2.convert(disk, qcow2)
    rebuilt = read_qcow2(qcow2)
    original = disk.read_bytes()
    if rebuilt != original:
        where = next((i for i in range(len(original)) if rebuilt[i:i + 1] != original[i:i + 1]), None)
        failures.append(f"qcow2: the image does not reconstruct the raw disk "
                        f"(first difference at byte {where})")
    if qcow2.stat().st_size >= disk.stat().st_size:
        failures.append("qcow2: the sparse image is no smaller than the raw disk")

    print(f"test-disk: GPT and qcow2 verified, "
          f"{qcow2.stat().st_size} bytes for a {report['bytes']} byte disk")


def test_iso(work, root_image, esp_image, failures):
    iso = work / "losos.iso"
    report = mkiso.build(esp_image, root_image, iso, "LOSOS", "root-x86-64",
                         free_bytes=0, seed="test")

    volume = read_iso(iso)
    if volume["volume_id"] != "LOSOS":
        failures.append(f"iso: volume id is {volume['volume_id']!r}")
    if set(volume["entries"]) != {"ESP.IMG;1", "ROOT.IMG;1"}:
        failures.append(f"iso: root directory holds {sorted(volume['entries'])}")

    table = read_gpt(iso)
    esp = table["partitions"][0]
    if esp["type"] != mkgpt.TYPES["esp"]:
        failures.append("iso: the first partition is not an ESP")

    # The whole point of the hybrid: one copy of the bytes, three pointers.
    iso_extent = volume["entries"]["ESP.IMG;1"][0]
    if iso_extent * mkiso.ISO_SECTOR != esp["first_lba"] * SECTOR:
        failures.append(
            "iso: the ISO 9660 file and the GPT partition point at different "
            "places, so the medium would boot one and install the other"
        )
    if volume["boot_extent"] != iso_extent:
        failures.append("iso: the boot catalog does not point at the ESP image")

    with open(iso, "rb") as image:
        image.seek(esp["first_lba"] * SECTOR)
        head = image.read(512)
    if head[82:90] != b"FAT32   ":
        failures.append("iso: the ESP partition does not start with a FAT32 volume")

    print(f"test-disk: hybrid ISO verified, {report['bytes']} bytes, "
          "El Torito and GPT agree on where the ESP is")


def main():
    failures = []
    with tempfile.TemporaryDirectory(prefix="losos-disk-") as temporary:
        work = Path(temporary)
        root_image, _ = test_ext4(work, failures)
        test_ext4_resize_inode(work, failures)
        test_ext4_extent_index(work, failures)
        esp_image = test_fat(work, failures)
        test_gpt_and_qcow2(work, root_image, esp_image, failures)
        test_iso(work, root_image, esp_image, failures)

    if failures:
        print("\ntest-disk: FAILED", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1
    print("test-disk: green")
    return 0


if __name__ == "__main__":
    sys.exit(main())
