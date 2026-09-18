#!/usr/bin/env python3
"""Write an ext4 filesystem image from a directory tree, without mkfs.

`mkfs` is not in pm's fingerprint table (C2), `losetup` and `mount` are not
either, and the jail has no privileges to use them with anyway (C7). So the
root filesystem is written here, the same way the initramfs and the UKI are:
a format writer in stdlib-only Python, invoked as `python3 <script>`, which is
the one shape pm will run.

This is the largest piece of novel code in the tree and the one whose bugs
surface as an unbootable disk rather than a failed build, so two things are
deliberate. The feature set is the smallest that a current kernel mounts
read-write and that `e2fsck` calls clean -- extents, filetype, a journal, and
nothing else. And every structure it writes is parsed back by an independent
reader in `tools/gates/test-disk.py`, written against the on-disk layout rather
than by reusing anything here.

What is deliberately NOT enabled, each because leaving it out removes code that
could be wrong without removing anything the OS needs:

  metadata_csum   every block would carry a crc32c this would have to compute
                  correctly in a dozen places; a wrong one is a filesystem the
                  kernel refuses to mount. ext4 without it is ordinary ext4.
  dir_index       hashed directories are a lookup optimisation. Linear
                  directories are correct, and the kernel adds the index
                  itself the first time it writes to a large directory.
  flex_bg         groups metadata from several block groups together to make
                  allocation contiguous. This image is written once and never
                  allocated into, so it buys nothing.
  bigalloc        changes the unit of allocation everywhere.

`resize_inode` IS enabled, and that one is not optional: `systemd-repart` grows
the root partition to fill the disk on first boot and then grows the filesystem
into it, and an online grow past the block groups the current descriptor table
can address needs the reserved GDT blocks this inode anchors. Without it the
filesystem silently stops growing at the first size boundary.

Everything is reproducible: uid and gid are flattened to 0 and every timestamp
is the epoch, exactly as the rootfs tarball does it, and the filesystem UUID is
derived from the tree rather than drawn from urandom.
"""

import argparse
import hashlib
import os
import stat
import struct
import sys
from pathlib import Path

BLOCK_SIZE = 4096
INODE_SIZE = 256
# The inode's fixed part is 128 bytes; i_extra_isize describes how much of the
# rest is more inode rather than in-inode extended attributes.
EXTRA_ISIZE = 32
DESC_SIZE = 32
# One block bitmap block covers one group, so a group is as many blocks as
# there are bits in a block.
BLOCKS_PER_GROUP = BLOCK_SIZE * 8
SECTORS_PER_BLOCK = BLOCK_SIZE // 512

EXT4_MAGIC = 0xEF53
ROOT_INO = 2
RESIZE_INO = 7
JOURNAL_INO = 8
LOST_FOUND_INO = 11
FIRST_INO = 11

# Feature bits. The comment above says why this set and no more.
COMPAT_HAS_JOURNAL = 0x0004
COMPAT_EXT_ATTR = 0x0008
COMPAT_RESIZE_INODE = 0x0010
INCOMPAT_FILETYPE = 0x0002
INCOMPAT_EXTENTS = 0x0040
RO_COMPAT_SPARSE_SUPER = 0x0001
RO_COMPAT_LARGE_FILE = 0x0002
RO_COMPAT_HUGE_FILE = 0x0008
RO_COMPAT_DIR_NLINK = 0x0020
RO_COMPAT_EXTRA_ISIZE = 0x0040

INODE_FL_EXTENTS = 0x00080000

EXTENT_MAGIC = 0xF30A
# An extent's length field is 16 bits and the top bit marks the extent
# uninitialised, so one extent maps at most 32768 blocks.
MAX_EXTENT_LEN = 32768
EXTENT_HEADER = 12
EXTENT_ENTRY = 12
# The 60 bytes of i_block hold a header and four entries exactly.
INLINE_EXTENTS = (60 - EXTENT_HEADER) // EXTENT_ENTRY
EXTENTS_PER_BLOCK = (BLOCK_SIZE - EXTENT_HEADER) // EXTENT_ENTRY

FT_REG, FT_DIR, FT_CHRDEV, FT_BLKDEV, FT_FIFO, FT_SOCK, FT_SYMLINK = 1, 2, 3, 4, 5, 6, 7

XATTR_MAGIC = 0xEA020000
# ext4 stores a name prefix as an index rather than as text. Longest first, so
# `system.posix_acl_access` is not matched by the bare `system.` prefix.
XATTR_PREFIXES = [
    (2, "system.posix_acl_access"),
    (3, "system.posix_acl_default"),
    (1, "user."),
    (4, "trusted."),
    (6, "security."),
    (7, "system."),
]

JBD2_MAGIC = 0xC03B3998
JBD2_SUPERBLOCK_V2 = 4


def align(value, to):
    return (value + to - 1) // to * to


def divide_up(value, by):
    return (value + by - 1) // by


class Node:
    """One filesystem object, with the numbers the writer needs about it."""

    __slots__ = (
        "path", "name", "ino", "mode", "uid", "gid", "size", "kind", "rdev",
        "target", "parent", "children", "entries", "xattrs", "block_count",
        "first_block", "dir_blocks", "extent_blocks", "xattr_block", "links",
    )

    def __init__(self, path, name, ino, st, parent):
        self.path = path
        self.name = name
        self.ino = ino
        self.mode = st.st_mode
        # Flattened to root, exactly as the rootfs tarball is
        # (`--owner=0 --group=0 --numeric-owner`). Nothing in the staged tree
        # carries meaningful ownership: pm's jail is a user namespace mapping
        # only uid 0, so every file in it is already root-owned, and the OS
        # creates its own users at first boot from sysusers.d.
        self.uid = 0
        self.gid = 0
        self.size = 0
        self.kind = None
        self.rdev = 0
        self.target = b""
        self.parent = parent
        # Child nodes, which decide a directory's link count, and the names it
        # lists, which are not the same list: a hard link adds a name without
        # adding a node.
        self.children = []
        self.entries = []
        self.xattrs = []
        self.block_count = 0
        self.first_block = 0
        self.dir_blocks = []
        self.extent_blocks = []
        self.xattr_block = 0
        self.links = 1


def read_xattrs(path):
    """Every extended attribute on `path`, as (name_index, suffix, value).

    Covers POSIX ACLs and file capabilities, which are xattrs like any other.
    Dropping them would produce a filesystem that mounts and behaves subtly
    differently from the tree it was built from -- a binary that silently loses
    its capabilities, a directory that loses its default ACL.
    """
    try:
        names = os.listxattr(path, follow_symlinks=False)
    except OSError:
        # Not every filesystem the build runs on supports them, and a tree
        # with none is the normal case rather than an error.
        return []
    found = []
    for name in sorted(names):
        try:
            value = os.getxattr(path, name, follow_symlinks=False)
        except OSError:
            continue
        for index, prefix in XATTR_PREFIXES:
            if name == prefix or (prefix.endswith(".") and name.startswith(prefix)):
                suffix = name[len(prefix):] if prefix.endswith(".") else ""
                found.append((index, suffix.encode(), value))
                break
        else:
            # An unknown prefix has no index, and ext4 has nowhere to put the
            # full name. Say so rather than writing something that reads back
            # as a different attribute.
            print(f"mkext4: dropping unrepresentable xattr {name} on {path}",
                  file=sys.stderr)
    return found


def xattr_hash(name, value):
    """ext4's per-entry attribute hash (`ext4_xattr_hash_entry`).

    e2fsck validates it, so it is not optional even though nothing here shares
    an attribute block between inodes.
    """
    hashed = 0
    for byte in name:
        hashed = ((hashed << 5) ^ (hashed >> 27) ^ byte) & 0xFFFFFFFF
    padded = value + b"\0" * ((-len(value)) % 4)
    for offset in range(0, len(padded), 4):
        word = struct.unpack_from("<I", padded, offset)[0]
        hashed = ((hashed << 16) ^ (hashed >> 16) ^ word) & 0xFFFFFFFF
    return hashed


def xattr_entries(attributes, value_base):
    """Serialise attribute entries and their values.

    Returns (entries, values, value_region_length). Entries grow forward from
    the start of the region and values grow backward from its end, which is why
    the caller has to know how big the region is before this can be called:
    `value_base` is the length of the area the offsets are measured in.
    """
    entries = bytearray()
    values = bytearray()
    placed = []
    offset = value_base
    for index, name, value in attributes:
        offset -= align(len(value), 4)
        placed.append((index, name, value, offset))
        values[0:0] = value + b"\0" * ((-len(value)) % 4)
    for index, name, value, offset in placed:
        entries += struct.pack(
            "<BBHIII", len(name), index, offset, 0, len(value),
            xattr_hash(name, value),
        )
        entries += name
        entries += b"\0" * ((-len(entries)) % 4)
    return bytes(entries), bytes(values), offset


class Ext4Builder:
    def __init__(self, root, label="", uuid=None, size=None, journal_blocks=None):
        self.root = Path(root).resolve()
        self.label = label
        self.uuid = uuid
        self.requested_size = size
        self.requested_journal = journal_blocks
        self.nodes = {}
        self.order = []
        # (device, inode) of an already-seen hard link target, so the second
        # name for a file gets the first one's inode rather than a copy.
        self.hard_links = {}
        self.next_ino = FIRST_INO + 1

    # ---------------------------------------------------------------- scan

    def scan(self):
        st = self.root.lstat()
        root = Node(self.root, "", ROOT_INO, st, ROOT_INO)
        root.kind = FT_DIR
        root.xattrs = read_xattrs(self.root)
        self.nodes[ROOT_INO] = root
        self.order.append(root)

        # lost+found is not decoration: e2fsck puts disconnected inodes in it,
        # and creating it here means a recovery never has to allocate a
        # directory in a filesystem it is in the middle of repairing.
        lost = Node(self.root, "lost+found", LOST_FOUND_INO, st, ROOT_INO)
        lost.kind = FT_DIR
        lost.mode = stat.S_IFDIR | 0o700
        self.nodes[LOST_FOUND_INO] = lost
        self.order.append(lost)
        root.children.append(lost)
        root.entries.append(("lost+found", LOST_FOUND_INO, FT_DIR))

        self._scan_directory(self.root, root)

    def _scan_directory(self, path, parent):
        for entry in sorted(os.scandir(path), key=lambda e: e.name):
            st = entry.stat(follow_symlinks=False)
            mode = st.st_mode

            if stat.S_ISREG(mode) and st.st_nlink > 1:
                key = (st.st_dev, st.st_ino)
                if key in self.hard_links:
                    existing = self.hard_links[key]
                    existing.links += 1
                    parent.entries.append((entry.name, existing.ino, existing.kind))
                    continue

            node = Node(Path(entry.path), entry.name, self.next_ino, st, parent.ino)
            self.next_ino += 1
            node.xattrs = read_xattrs(entry.path)

            if stat.S_ISDIR(mode):
                node.kind = FT_DIR
            elif stat.S_ISREG(mode):
                node.kind = FT_REG
                node.size = st.st_size
                if st.st_nlink > 1:
                    self.hard_links[(st.st_dev, st.st_ino)] = node
            elif stat.S_ISLNK(mode):
                node.kind = FT_SYMLINK
                node.target = os.readlink(entry.path).encode()
                node.size = len(node.target)
            elif stat.S_ISCHR(mode):
                node.kind, node.rdev = FT_CHRDEV, st.st_rdev
            elif stat.S_ISBLK(mode):
                node.kind, node.rdev = FT_BLKDEV, st.st_rdev
            elif stat.S_ISFIFO(mode):
                node.kind = FT_FIFO
            elif stat.S_ISSOCK(mode):
                node.kind = FT_SOCK
            else:
                print(f"mkext4: skipping {entry.path}: unknown type",
                      file=sys.stderr)
                continue

            self.nodes[node.ino] = node
            self.order.append(node)
            parent.children.append(node)
            parent.entries.append((node.name, node.ino, node.kind))

            if node.kind == FT_DIR:
                self._scan_directory(Path(entry.path), node)

    # ----------------------------------------------------------- directories

    def build_directories(self):
        """Turn each directory's children into the blocks that hold them.

        Done before any block is allocated, because a directory's size is what
        decides how many blocks it needs and the geometry depends on the total.
        """
        for node in self.order:
            if node.kind != FT_DIR:
                continue
            entries = [(node.ino, b".", FT_DIR), (node.parent, b"..", FT_DIR)]
            for name, ino, kind in node.entries:
                entries.append((ino, name.encode(), kind))
            node.dir_blocks = self._pack_directory(entries)
            node.block_count = len(node.dir_blocks)

    @staticmethod
    def _pack_directory(entries):
        """Pack directory entries into blocks.

        An entry never straddles a block: the last one in each block has its
        rec_len stretched to the block's end, which is also how a directory
        with a hole in it is read back.
        """
        blocks = []
        current = []
        used = 0
        for ino, name, kind in entries:
            need = align(8 + len(name), 4)
            if used + need > BLOCK_SIZE:
                blocks.append((current, used))
                current, used = [], 0
            current.append((ino, name, kind, need))
            used += need
        blocks.append((current, used))

        packed = []
        for items, _ in blocks:
            block = bytearray()
            for index, (ino, name, kind, need) in enumerate(items):
                last = index == len(items) - 1
                rec_len = BLOCK_SIZE - len(block) if last else need
                block += struct.pack("<IHBB", ino, rec_len, len(name), kind)
                block += name
                block += b"\0" * (rec_len - 8 - len(name))
            packed.append(bytes(block))
        return packed

    # -------------------------------------------------------------- geometry

    def plan(self):
        """Work out how many blocks and inodes the image needs, then lay it out.

        Geometry is circular -- the metadata per group depends on the group
        count, which depends on the total size, which depends on how much
        metadata there is -- so it is solved by iterating rather than by
        algebra. It converges in two or three rounds because each round only
        has to cover the shortfall the last one reported.
        """
        data_blocks = 0
        for node in self.order:
            if node.kind == FT_DIR:
                node.block_count = len(node.dir_blocks)
            elif node.kind == FT_REG:
                node.block_count = divide_up(node.size, BLOCK_SIZE)
            elif node.kind == FT_SYMLINK and len(node.target) >= 60:
                # A short target lives in i_block itself; only a long one needs
                # a block of its own.
                node.block_count = 1
            else:
                node.block_count = 0

            node.extent_blocks = []
            if node.block_count:
                extents = divide_up(node.block_count, MAX_EXTENT_LEN)
                if extents > INLINE_EXTENTS:
                    node.extent_blocks = [0] * divide_up(extents, EXTENTS_PER_BLOCK)
            data_blocks += node.block_count + len(node.extent_blocks)
            if self._xattr_needs_block(node):
                data_blocks += 1

        self.inode_count_used = len(self.nodes) + FIRST_INO - 1

        # The journal is sized the way mke2fs sizes it: enough to hold a
        # handful of full transactions, not a fraction of the device.
        journal = self.requested_journal
        if journal is None:
            journal = 1024 if data_blocks < 32768 else 4096
        if journal < 1024:
            # jbd2 refuses a journal smaller than this, and so does e2fsck --
            # the failure is "the journal superblock is corrupt", which points
            # at the contents rather than at the size.
            sys.exit(f"mkext4: a journal of {journal} blocks is below jbd2's minimum of 1024")
        self.journal_blocks = journal
        data_blocks += journal

        # One inode per 16KB of filesystem is mke2fs's default ratio, and it is
        # what decides whether the installed system can create files at all
        # after it grows: the inode count is fixed at mkfs time and growing the
        # filesystem does not add any.
        total = self.requested_size
        if total is None:
            # A quarter again as much as the tree needs, so the image has room
            # for the first boot's writes before repart grows it.
            estimate = int(data_blocks * 1.25) + 1024
            total = max(estimate, 16384)
        else:
            total = total // BLOCK_SIZE

        for _ in range(8):
            geometry = self._geometry(total)
            if geometry["free_blocks"] >= data_blocks:
                break
            total += data_blocks - geometry["free_blocks"] + geometry["groups"] * 4
        else:
            sys.exit("mkext4: could not find a geometry that fits the tree")

        self.total_blocks = total
        self.geometry = geometry
        if geometry["inodes_total"] < self.inode_count_used:
            sys.exit(
                f"mkext4: {self.inode_count_used} inodes needed but the "
                f"geometry only provides {geometry['inodes_total']}"
            )

    def _xattr_needs_block(self, node):
        """Whether this inode's attributes are too big to live inside it.

        The in-inode area is whatever is left of a 256-byte inode after the
        fixed 128 bytes and i_extra_isize -- 92 usable bytes after the magic.
        Capabilities and a small ACL fit; a long default ACL does not.
        """
        if not node.xattrs:
            return False
        return self._xattr_size(node.xattrs) > INODE_SIZE - 128 - EXTRA_ISIZE - 4

    @staticmethod
    def _xattr_size(attributes):
        size = 0
        for _, name, value in attributes:
            size += align(16 + len(name), 4) + align(len(value), 4)
        return size + 4  # the terminating zero entry

    def _geometry(self, total_blocks):
        groups = divide_up(total_blocks, BLOCKS_PER_GROUP)
        gdt_blocks = divide_up(groups * DESC_SIZE, BLOCK_SIZE)

        # Room to grow. systemd-repart gives the root partition whatever is
        # left of the disk on first boot and then grows the filesystem into it,
        # and each further block group needs a descriptor: without spare
        # descriptor blocks the grow stops at the first boundary, reports
        # success for the part it managed, and leaves the rest of the disk
        # unused. A thousandfold is what mke2fs reserves for, and the ceiling
        # is the format's rather than a choice -- the resize inode addresses
        # descriptor blocks through one double-indirect block, so there are
        # only as many slots as a block holds pointers.
        grown = divide_up(total_blocks * 1024, BLOCKS_PER_GROUP)
        slots = BLOCK_SIZE // 4
        reserved_gdt = divide_up(grown * DESC_SIZE, BLOCK_SIZE) - gdt_blocks
        reserved_gdt = max(0, min(reserved_gdt, slots - gdt_blocks))

        inodes_total = max(divide_up(total_blocks * BLOCK_SIZE, 16384),
                           self.inode_count_used + 16)
        inodes_per_group = divide_up(inodes_total, groups)
        # The bitmap is a byte array, and a group's inode table has to be a
        # whole number of blocks.
        inodes_per_group = align(inodes_per_group, BLOCK_SIZE // INODE_SIZE)
        inodes_per_group = min(inodes_per_group, BLOCK_SIZE * 8)
        itable_blocks = inodes_per_group * INODE_SIZE // BLOCK_SIZE

        free = 0
        overheads = []
        for group in range(groups):
            first = group * BLOCKS_PER_GROUP
            last = min(first + BLOCKS_PER_GROUP, total_blocks)
            overhead = 2 + itable_blocks  # block bitmap, inode bitmap, table
            if has_super(group):
                overhead += 1 + gdt_blocks + reserved_gdt
            overheads.append(overhead)
            free += (last - first) - overhead
            if last - first <= overhead:
                sys.exit("mkext4: a block group has no room for data")

        return {
            "groups": groups,
            "gdt_blocks": gdt_blocks,
            "reserved_gdt": reserved_gdt,
            "inodes_per_group": inodes_per_group,
            "inodes_total": inodes_per_group * groups,
            "itable_blocks": itable_blocks,
            "overheads": overheads,
            "free_blocks": free,
        }

    # ------------------------------------------------------------ allocation

    def allocate(self):
        """Hand out every data block, in group order, after each group's metadata.

        Allocation is sequential and never freed, so a file's blocks are always
        contiguous and its extent list is as short as the format allows.
        """
        geometry = self.geometry
        self.group_start = []
        self.free_run = []
        for group in range(geometry["groups"]):
            first = group * BLOCKS_PER_GROUP
            last = min(first + BLOCKS_PER_GROUP, self.total_blocks)
            cursor = first
            if has_super(group):
                cursor += 1 + geometry["gdt_blocks"] + geometry["reserved_gdt"]
            self.group_start.append(cursor)
            self.free_run.append((cursor + 2 + geometry["itable_blocks"], last))

        self.cursor_group = 0
        self.used_blocks = set()

        # The journal first, so it lands early on the disk where a rotating
        # device would seek least. It costs nothing on flash and is what
        # mke2fs does.
        self.journal_start = self._take(self.journal_blocks)

        for node in self.order:
            if self._xattr_needs_block(node):
                node.xattr_block = self._take(1)
            if node.block_count:
                node.first_block = self._take(node.block_count)
            if node.extent_blocks:
                start = self._take(len(node.extent_blocks))
                node.extent_blocks = list(
                    range(start, start + len(node.extent_blocks))
                )

    def _take(self, count):
        """Allocate `count` contiguous blocks.

        Contiguous is not merely nice here: an extent cannot describe a gap, so
        a run that crossed a group's metadata would need a second extent and
        the caller's inline extent list could overflow. Groups are therefore
        skipped whole rather than packed.
        """
        while self.cursor_group < len(self.free_run):
            start, end = self.free_run[self.cursor_group]
            if end - start >= count:
                self.free_run[self.cursor_group] = (start + count, end)
                return start
            self.cursor_group += 1
        sys.exit(f"mkext4: out of contiguous space for {count} blocks")

    # --------------------------------------------------------------- writing

    def write(self, output):
        self.image = open(output, "wb+")
        try:
            # A sparse file: the holes cost nothing on disk and compress to
            # nothing in the .raw.xz that ships.
            self.image.truncate(self.total_blocks * BLOCK_SIZE)
            self._write_data()
            self._write_journal()
            self._write_inodes()
            self._write_bitmaps()
            self._write_superblocks()
        finally:
            self.image.close()

    def _put(self, block, data):
        self.image.seek(block * BLOCK_SIZE)
        self.image.write(data)

    def _write_data(self):
        for node in self.order:
            if node.kind == FT_DIR:
                for index, block in enumerate(node.dir_blocks):
                    self._put(node.first_block + index, block)
            elif node.kind == FT_REG and node.block_count:
                self.image.seek(node.first_block * BLOCK_SIZE)
                with open(node.path, "rb") as source:
                    remaining = node.size
                    while remaining > 0:
                        chunk = source.read(min(1 << 20, remaining))
                        if not chunk:
                            sys.exit(f"mkext4: {node.path} shrank while reading")
                        self.image.write(chunk)
                        remaining -= len(chunk)
                # The tail of the last block has to be zero rather than
                # whatever the sparse file had there.
                tail = (-node.size) % BLOCK_SIZE
                if tail:
                    self.image.write(b"\0" * tail)
            elif node.kind == FT_SYMLINK and node.block_count:
                self._put(node.first_block,
                          node.target + b"\0" * (BLOCK_SIZE - len(node.target)))

            if node.xattr_block:
                self._put(node.xattr_block, self._xattr_block_bytes(node))

    def _xattr_block_bytes(self, node):
        """A whole block holding one inode's attributes.

        Never shared between inodes: sharing is what h_refcount and the block
        hash are for, and an image written once has nothing to gain from it.
        """
        header_len = 32
        entries, values, _ = xattr_entries(node.xattrs, BLOCK_SIZE - header_len)
        block = bytearray(BLOCK_SIZE)
        combined = 0
        for _, name, value in node.xattrs:
            combined = (
                (combined << 16) ^ (combined >> 16) ^ xattr_hash(name, value)
            ) & 0xFFFFFFFF
        struct.pack_into("<IIIII", block, 0, XATTR_MAGIC, 1, 1, combined, 0)
        block[header_len:header_len + len(entries)] = entries
        block[BLOCK_SIZE - len(values):] = values
        return bytes(block)

    def _write_journal(self):
        """An empty, clean JBD2 journal.

        Big-endian, unlike everything else on an ext4 filesystem: jbd2 predates
        the decision and keeps its own byte order. s_start=0 with s_sequence=1
        is what the kernel reads as "nothing to replay".
        """
        superblock = bytearray(BLOCK_SIZE)
        struct.pack_into(
            ">III", superblock, 0, JBD2_MAGIC, JBD2_SUPERBLOCK_V2, 0,
        )
        struct.pack_into(
            ">IIIII", superblock, 12,
            BLOCK_SIZE,             # s_blocksize
            self.journal_blocks,    # s_maxlen
            1,                      # s_first: block 0 is this superblock
            1,                      # s_sequence
            0,                      # s_start: an empty journal
        )
        superblock[48:64] = self.fs_uuid
        struct.pack_into(">I", superblock, 64, 1)  # s_nr_users
        self._put(self.journal_start, bytes(superblock))
        # The rest of the journal is already zero in the sparse image, and a
        # zeroed block is not a valid descriptor, which is exactly what an
        # empty journal needs it to be.

    # ---------------------------------------------------------------- inodes

    def _extent_body(self, node):
        """i_block for a file, plus the contents of any index block it needs."""
        blocks = node.block_count
        extents = []
        logical = 0
        physical = node.first_block
        while blocks > 0:
            length = min(blocks, MAX_EXTENT_LEN)
            extents.append((logical, length, physical))
            logical += length
            physical += length
            blocks -= length

        def leaf(entry):
            logical, length, physical = entry
            # A length of exactly 32768 is stored as 32768 with the top bit
            # clear only because the extent is initialised; an uninitialised
            # one would set it and halve the range. Nothing here writes one.
            return struct.pack("<IHHI", logical, length,
                               (physical >> 32) & 0xFFFF, physical & 0xFFFFFFFF)

        if not extents:
            body = struct.pack("<HHHHI", EXTENT_MAGIC, 0, INLINE_EXTENTS, 0, 0)
            return body + b"\0" * (60 - len(body)), []

        if len(extents) <= INLINE_EXTENTS:
            body = struct.pack("<HHHHI", EXTENT_MAGIC, len(extents),
                               INLINE_EXTENTS, 0, 0)
            body += b"".join(leaf(entry) for entry in extents)
            return body + b"\0" * (60 - len(body)), []

        # One level of index blocks. Four index entries in the inode, each
        # naming a block of up to 340 extents: enough for a file of 178TB,
        # which is not a limit this image will meet.
        chunks = [extents[i:i + EXTENTS_PER_BLOCK]
                  for i in range(0, len(extents), EXTENTS_PER_BLOCK)]
        if len(chunks) > INLINE_EXTENTS:
            sys.exit(f"mkext4: {node.path} needs a deeper extent tree than this writes")

        contents = []
        index = struct.pack("<HHHHI", EXTENT_MAGIC, len(chunks), INLINE_EXTENTS, 1, 0)
        for chunk, block in zip(chunks, node.extent_blocks):
            index += struct.pack("<IIHH", chunk[0][0], block & 0xFFFFFFFF,
                                 (block >> 32) & 0xFFFF, 0)
            leaves = struct.pack("<HHHHI", EXTENT_MAGIC, len(chunk),
                                 EXTENTS_PER_BLOCK, 0, 0)
            leaves += b"".join(leaf(entry) for entry in chunk)
            contents.append((block, leaves + b"\0" * (BLOCK_SIZE - len(leaves))))
        return index + b"\0" * (60 - len(index)), contents

    def _inode_bytes(self, node):
        raw = bytearray(INODE_SIZE)
        flags = 0
        body = b"\0" * 60
        index_blocks = []
        metadata_blocks = 0

        if node.kind == FT_SYMLINK and len(node.target) < 60:
            # A fast symlink: the target sits in i_block instead of a block.
            body = node.target + b"\0" * (60 - len(node.target))
        elif node.kind in (FT_CHRDEV, FT_BLKDEV):
            major, minor = os.major(node.rdev), os.minor(node.rdev)
            if major < 256 and minor < 256:
                body = struct.pack("<I", (major << 8) | minor) + b"\0" * 56
            else:
                # The second encoding, for numbers that do not fit the first.
                packed = (minor & 0xFF) | (major << 8) | ((minor & ~0xFF) << 12)
                body = struct.pack("<II", 0, packed) + b"\0" * 52
        elif node.kind in (FT_FIFO, FT_SOCK):
            body = b"\0" * 60
        else:
            flags |= INODE_FL_EXTENTS
            body, index_blocks = self._extent_body(node)
            metadata_blocks = len(index_blocks)

        size = node.size if node.kind != FT_DIR else node.block_count * BLOCK_SIZE
        charged = node.block_count + metadata_blocks
        if node.xattr_block:
            charged += 1
        sectors = charged * SECTORS_PER_BLOCK

        links = node.links
        if node.kind == FT_DIR:
            # "." plus the entry in the parent, plus one per subdirectory's "..".
            links = 2 + sum(1 for child in node.children if child.kind == FT_DIR)

        struct.pack_into(
            "<HHIIIIIHHII", raw, 0,
            node.mode & 0xFFFF,
            node.uid,
            size & 0xFFFFFFFF,
            0,                  # i_atime: the epoch, as in the rootfs tarball
            0,                  # i_ctime
            0,                  # i_mtime
            0,                  # i_dtime
            node.gid,
            links,
            sectors & 0xFFFFFFFF,
            flags,
        )
        raw[0x28:0x28 + 60] = body
        struct.pack_into("<I", raw, 0x68, node.xattr_block)  # i_file_acl_lo
        if node.kind == FT_REG:
            struct.pack_into("<I", raw, 0x6C, node.size >> 32)  # i_size_high
        struct.pack_into("<H", raw, 0x74, (sectors >> 32) & 0xFFFF)
        struct.pack_into("<H", raw, 0x80, EXTRA_ISIZE)

        if node.xattrs and not node.xattr_block:
            region = INODE_SIZE - 128 - EXTRA_ISIZE
            entries, values, _ = xattr_entries(node.xattrs, region - 4)
            start = 128 + EXTRA_ISIZE
            struct.pack_into("<I", raw, start, XATTR_MAGIC)
            raw[start + 4:start + 4 + len(entries)] = entries
            if values:
                raw[INODE_SIZE - len(values):] = values

        return bytes(raw), index_blocks

    def _write_inodes(self):
        geometry = self.geometry
        self.used_dirs = [0] * geometry["groups"]
        self.used_inodes = set()

        for node in self.order:
            raw, index_blocks = self._inode_bytes(node)
            for block, contents in index_blocks:
                self._put(block, contents)
            self._put_inode(node.ino, raw)
            self.used_inodes.add(node.ino)
            if node.kind == FT_DIR:
                self.used_dirs[(node.ino - 1) // geometry["inodes_per_group"]] += 1

        # The reserved inodes below FIRST_INO exist whether or not anything
        # uses them; e2fsck expects them present and in use.
        for ino in range(1, FIRST_INO):
            if ino in self.used_inodes:
                continue
            raw = bytearray(INODE_SIZE)
            struct.pack_into("<H", raw, 0x80, EXTRA_ISIZE)
            if ino == JOURNAL_INO:
                raw = bytearray(self._journal_inode())
            elif ino == RESIZE_INO:
                raw = bytearray(self._resize_inode())
            self._put_inode(ino, bytes(raw))
            self.used_inodes.add(ino)

    @staticmethod
    def _inline_extents(first_block, block_count, what):
        """An extent list for a contiguous run, small enough to live in i_block.

        Callers that can outgrow four extents build an index block instead;
        this is for the ones that cannot, and it says so rather than writing
        past the end of the 60 bytes it has.
        """
        extents = []
        logical, physical, blocks = 0, first_block, block_count
        while blocks > 0:
            length = min(blocks, MAX_EXTENT_LEN)
            extents.append(struct.pack("<IHHI", logical, length,
                                       (physical >> 32) & 0xFFFF,
                                       physical & 0xFFFFFFFF))
            logical += length
            physical += length
            blocks -= length
        if len(extents) > INLINE_EXTENTS:
            sys.exit(
                f"mkext4: {what} needs {len(extents)} extents and only "
                f"{INLINE_EXTENTS} fit in an inode"
            )
        body = struct.pack("<HHHHI", EXTENT_MAGIC, len(extents), INLINE_EXTENTS, 0, 0)
        body += b"".join(extents)
        return body + b"\0" * (60 - len(body))

    def _journal_inode(self):
        raw = bytearray(INODE_SIZE)
        sectors = self.journal_blocks * SECTORS_PER_BLOCK
        struct.pack_into(
            "<HHIIIIIHHII", raw, 0,
            stat.S_IFREG | 0o600, 0,
            self.journal_blocks * BLOCK_SIZE,
            0, 0, 0, 0, 0,
            1,                              # i_links_count
            sectors, INODE_FL_EXTENTS,
        )
        raw[0x28:0x28 + 60] = self._inline_extents(
            self.journal_start, self.journal_blocks, "the journal"
        )
        struct.pack_into("<H", raw, 0x80, EXTRA_ISIZE)
        return bytes(raw)

    def _resize_inode(self):
        """The inode that owns the reserved group-descriptor blocks.

        Its shape is peculiar and worth stating, because nothing else on the
        filesystem is laid out this way. The inode's double-indirect slot
        points at one block whose entries are the reserved GDT blocks in group
        0. Each of those blocks is then read as an indirect block, and its
        entries are that same block's backup copies in every group that carries
        a superblock backup. So growing the filesystem finds every copy of
        every descriptor block it has to update by walking one inode, without
        having to know where the backups are.

        The indexing is the part that is easy to get wrong and impossible to
        guess: a reserved block is not stored at the next free slot but at the
        slot its own block number names, counting the real descriptor blocks
        first. Descriptor block N -- reserved or not -- is slot N-1, so the
        slots belonging to the descriptor table that already exists stay empty.
        Filling from slot zero instead produces a structure that reads back
        perfectly and that e2fsck rejects outright.

        This is the one structure here that uses the old block map rather than
        extents: the resize inode predates extents and the kernel reads it that
        way regardless of what the inode's flags say.
        """
        raw = bytearray(INODE_SIZE)
        geometry = self.geometry
        reserved = geometry["reserved_gdt"]

        pointers = bytearray(BLOCK_SIZE)
        charged = 0
        if reserved:
            first_reserved = 1 + geometry["gdt_blocks"]
            for index in range(reserved):
                gdt_block = first_reserved + index
                struct.pack_into("<I", pointers, (gdt_block - 1) * 4, gdt_block)
                charged += 1
                # The reserved block itself, read as an indirect block, lists
                # its own backups.
                backups = bytearray(BLOCK_SIZE)
                count = 0
                for group in range(1, geometry["groups"]):
                    if not has_super(group):
                        continue
                    struct.pack_into("<I", backups, count * 4,
                                     group * BLOCKS_PER_GROUP + gdt_block)
                    count += 1
                    charged += 1
                self._put(gdt_block, bytes(backups))
            self._put(self.dind_block, bytes(pointers))
            charged += 1

        sectors = charged * SECTORS_PER_BLOCK
        # i_size spans the whole double-indirect range rather than the part
        # that is used, which makes it a constant for a given block size. It is
        # what e2fsck recomputes and compares against.
        pointers_per_block = BLOCK_SIZE // 4
        span = 12 + pointers_per_block + pointers_per_block * pointers_per_block
        struct.pack_into(
            "<HHIIIIIHHII", raw, 0,
            stat.S_IFREG | 0o600, 0,
            (span * BLOCK_SIZE) & 0xFFFFFFFF,
            0, 0, 0, 0, 0,
            1,                               # i_links_count
            sectors, 0,
        )
        struct.pack_into("<I", raw, 0x6C, (span * BLOCK_SIZE) >> 32)
        if reserved:
            # i_block[EXT2_DIND_BLOCK]: twelve direct slots, then the single
            # indirect, then this one. Every other slot has to stay zero --
            # e2fsck treats a resize inode with anything else in it as invalid
            # rather than as a resize inode with extra blocks.
            struct.pack_into("<I", raw, 0x28 + 13 * 4, self.dind_block)
        struct.pack_into("<H", raw, 0x80, EXTRA_ISIZE)
        return bytes(raw)

    def _put_inode(self, ino, raw):
        geometry = self.geometry
        group = (ino - 1) // geometry["inodes_per_group"]
        index = (ino - 1) % geometry["inodes_per_group"]
        table = self.group_start[group] + 2
        self.image.seek(table * BLOCK_SIZE + index * INODE_SIZE)
        self.image.write(raw)

    # -------------------------------------------------------------- bitmaps

    def _write_bitmaps(self):
        geometry = self.geometry
        self.free_blocks_total = 0
        self.free_inodes_total = 0
        self.group_free_blocks = []
        self.group_free_inodes = []

        allocated = self._allocated_blocks()

        for group in range(geometry["groups"]):
            first = group * BLOCKS_PER_GROUP
            last = min(first + BLOCKS_PER_GROUP, self.total_blocks)
            bitmap = bytearray(BLOCK_SIZE)

            # Every block past the end of the filesystem is marked in use, so
            # nothing ever tries to allocate one.
            for offset in range(last - first, BLOCKS_PER_GROUP):
                bitmap[offset // 8] |= 1 << (offset % 8)

            used = 0
            for block in range(first, last):
                if allocated[block // 8] >> (block % 8) & 1:
                    offset = block - first
                    bitmap[offset // 8] |= 1 << (offset % 8)
                    used += 1

            self._put(self.group_start[group], bitmap)
            free = (last - first) - used
            self.group_free_blocks.append(free)
            self.free_blocks_total += free

            inode_bitmap = bytearray(BLOCK_SIZE)
            per_group = geometry["inodes_per_group"]
            for offset in range(per_group, BLOCK_SIZE * 8):
                inode_bitmap[offset // 8] |= 1 << (offset % 8)
            used_inodes = 0
            for index in range(per_group):
                ino = group * per_group + index + 1
                if ino in self.used_inodes:
                    inode_bitmap[index // 8] |= 1 << (index % 8)
                    used_inodes += 1
            self._put(self.group_start[group] + 1, inode_bitmap)
            self.group_free_inodes.append(per_group - used_inodes)
            self.free_inodes_total += per_group - used_inodes

    def _allocated_blocks(self):
        """A bitmap of every block the image has put something in.

        Derived from the layout that was actually written rather than tracked
        during allocation, so the group bitmaps cannot drift from the thing
        they describe. A bitmap rather than a set of block numbers because a
        full desktop image is a few million blocks, and a set of that many
        Python integers costs hundreds of megabytes to answer a question one
        bit each can answer.
        """
        geometry = self.geometry
        allocated = bytearray((self.total_blocks + 7) // 8)

        def mark(start, count):
            for block in range(start, start + count):
                allocated[block // 8] |= 1 << (block % 8)

        for group in range(geometry["groups"]):
            first = group * BLOCKS_PER_GROUP
            cursor = first
            if has_super(group):
                span = 1 + geometry["gdt_blocks"] + geometry["reserved_gdt"]
                mark(first, span)
                cursor = first + span
            mark(cursor, 2 + geometry["itable_blocks"])

        mark(self.journal_start, self.journal_blocks)

        for node in self.order:
            mark(node.first_block, node.block_count)
            for block in node.extent_blocks:
                mark(block, 1)
            if node.xattr_block:
                mark(node.xattr_block, 1)

        if geometry["reserved_gdt"]:
            mark(self.dind_block, 1)
        return allocated

    # ---------------------------------------------------------- superblocks

    def _superblock(self, group_number):
        geometry = self.geometry
        raw = bytearray(1024)
        struct.pack_into(
            "<IIIIIIIIIIII", raw, 0,
            geometry["inodes_total"],
            self.total_blocks,
            0,                          # s_r_blocks_count: none reserved
            self.free_blocks_total,
            self.free_inodes_total,
            0,                          # s_first_data_block
            2,                          # s_log_block_size: 1024 << 2
            2,                          # s_log_cluster_size
            BLOCKS_PER_GROUP,
            BLOCKS_PER_GROUP,           # s_clusters_per_group
            geometry["inodes_per_group"],
            0,                          # s_mtime
        )
        struct.pack_into("<I", raw, 0x30, 0)        # s_wtime
        struct.pack_into("<HH", raw, 0x34, 0, 0xFFFF)  # mnt_count, max_mnt_count
        struct.pack_into("<HHH", raw, 0x38, EXT4_MAGIC, 1, 1)  # magic, clean, continue
        struct.pack_into("<H", raw, 0x3E, 0)        # s_minor_rev_level
        struct.pack_into("<II", raw, 0x40, 0, 0)    # lastcheck, checkinterval
        struct.pack_into("<II", raw, 0x48, 0, 1)    # creator_os Linux, rev 1
        struct.pack_into("<HH", raw, 0x50, 0, 0)    # def_resuid, def_resgid
        struct.pack_into("<I", raw, 0x54, FIRST_INO)
        struct.pack_into("<HH", raw, 0x58, INODE_SIZE, group_number)

        compat = COMPAT_HAS_JOURNAL
        if any(node.xattrs for node in self.order):
            compat |= COMPAT_EXT_ATTR
        if geometry["reserved_gdt"]:
            compat |= COMPAT_RESIZE_INODE
        struct.pack_into("<III", raw, 0x5C, compat,
                         INCOMPAT_FILETYPE | INCOMPAT_EXTENTS,
                         RO_COMPAT_SPARSE_SUPER | RO_COMPAT_LARGE_FILE
                         | RO_COMPAT_HUGE_FILE | RO_COMPAT_DIR_NLINK
                         | RO_COMPAT_EXTRA_ISIZE)
        raw[0x68:0x78] = self.fs_uuid
        raw[0x78:0x88] = self.label.encode()[:16].ljust(16, b"\0")
        struct.pack_into("<H", raw, 0xCE, geometry["reserved_gdt"])
        struct.pack_into("<I", raw, 0xE0, JOURNAL_INO)
        # user_xattr and acl, the mount options the OS expects to be default.
        struct.pack_into("<I", raw, 0x100, 0x000C)
        struct.pack_into("<I", raw, 0x108, 0)       # s_mkfs_time
        struct.pack_into("<HH", raw, 0x15C, EXTRA_ISIZE, EXTRA_ISIZE)
        return bytes(raw)

    def _descriptors(self):
        geometry = self.geometry
        table = bytearray(geometry["gdt_blocks"] * BLOCK_SIZE)
        for group in range(geometry["groups"]):
            start = self.group_start[group]
            struct.pack_into(
                "<IIIHHHH", table, group * DESC_SIZE,
                start,                          # block bitmap
                start + 1,                      # inode bitmap
                start + 2,                      # inode table
                self.group_free_blocks[group],
                self.group_free_inodes[group],
                self.used_dirs[group],
                0,                              # bg_flags
            )
        return bytes(table)

    def _write_superblocks(self):
        geometry = self.geometry
        descriptors = self._descriptors()
        for group in range(geometry["groups"]):
            if not has_super(group):
                continue
            first = group * BLOCKS_PER_GROUP
            block = bytearray(BLOCK_SIZE)
            superblock = self._superblock(group)
            if group == 0:
                # The first 1024 bytes are left for a boot sector nobody uses
                # here, which is why block 0 holds the superblock at an offset
                # and every backup holds it at zero.
                block[1024:1024 + len(superblock)] = superblock
            else:
                block[0:len(superblock)] = superblock
            self._put(first, bytes(block))
            self.image.seek((first + 1) * BLOCK_SIZE)
            self.image.write(descriptors)

    # ------------------------------------------------------------------ run

    def build(self, output):
        self.scan()
        self.build_directories()
        self.plan()

        self.fs_uuid = self._derive_uuid()
        self.allocate()
        # The double-indirect block of the resize inode. Allocated last so it
        # does not disturb the contiguity of anything that matters, and only
        # when there is something for it to point at.
        self.dind_block = self._take(1) if self.geometry["reserved_gdt"] else 0

        self.write(output)
        return {
            "blocks": self.total_blocks,
            "bytes": self.total_blocks * BLOCK_SIZE,
            "inodes": self.geometry["inodes_total"],
            "used_inodes": len(self.used_inodes),
            "free_blocks": self.free_blocks_total,
            "uuid": format_uuid(self.fs_uuid),
        }

    def _derive_uuid(self):
        """A UUID that is a function of the tree, not of the clock.

        A filesystem UUID is normally random, which would make two builds of
        the same sources differ. Deriving it from what the filesystem contains
        keeps the image reproducible and still gives two different builds two
        different UUIDs, which is what anything looking one up needs.
        """
        if self.uuid:
            return bytes.fromhex(self.uuid.replace("-", ""))
        digest = hashlib.sha256()
        digest.update(b"losos-desktop/ext4/1\0")
        digest.update(self.label.encode() + b"\0")
        for node in self.order:
            digest.update(f"{node.ino}:{node.mode}:{node.size}:".encode())
            digest.update(node.name.encode() + b"\0")
        raw = bytearray(digest.digest()[:16])
        # Version 4, variant 1: not random, but shaped like a UUID so that
        # everything which parses one accepts it.
        raw[6] = (raw[6] & 0x0F) | 0x40
        raw[8] = (raw[8] & 0x3F) | 0x80
        return bytes(raw)


def has_super(group):
    """Whether a group carries a superblock backup, under sparse_super.

    Groups 0 and 1, and every group whose number is a power of 3, 5 or 7.
    """
    if group in (0, 1):
        return True
    for base in (3, 5, 7):
        power = base
        while power < group:
            power *= base
        if power == group:
            return True
    return False


def format_uuid(raw):
    hexed = raw.hex()
    return "-".join([hexed[0:8], hexed[8:12], hexed[12:16], hexed[16:20], hexed[20:32]])


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("root", help="directory tree to put in the filesystem")
    parser.add_argument("output", help="image file to write")
    parser.add_argument("--label", default="", help="volume label, up to 16 bytes")
    parser.add_argument("--uuid", help="filesystem UUID (default: derived from the tree)")
    parser.add_argument("--size", type=int,
                        help="image size in bytes (default: the tree plus a quarter)")
    parser.add_argument("--journal-blocks", type=int,
                        help="journal size in 4K blocks")
    args = parser.parse_args()

    root = Path(args.root)
    if not root.is_dir():
        sys.exit(f"mkext4: {root} is not a directory")

    builder = Ext4Builder(root, label=args.label, uuid=args.uuid,
                          size=args.size, journal_blocks=args.journal_blocks)
    report = builder.build(args.output)
    print(
        "mkext4: {bytes} bytes, {used_inodes}/{inodes} inodes, "
        "{free_blocks} blocks free, UUID {uuid}".format(**report)
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
