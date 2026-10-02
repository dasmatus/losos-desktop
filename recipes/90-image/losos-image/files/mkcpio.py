#!/usr/bin/env python3
"""Write a newc cpio archive, because `cpio` cannot be used here.

`cpio` is in pm's fingerprint table, and it is still dead weight: `cpio -o`
reads its file list from standard input, pm gives every build command
/dev/null on stdin (`run_jailed` sets `.stdin(Stdio::from(devnull()))`), and
there is no shell to write `find | cpio` with (C1, C4). GNU cpio has no option
to read that list from a file.

So the initramfs is written here instead. newc is a simple enough format that
this is about ninety lines, and it removes the last reason to want a shell
inside a build step.

Deliberately stdlib-only: this runs under the `python` fingerprint, which
grants no network, and anything it imported would have to be built first.
"""

import argparse
import os
import stat
import sys
from pathlib import Path

MAGIC = b"070701"
TRAILER = "TRAILER!!!"


def field(value):
    """newc stores every header field as 8 uppercase hex digits."""
    return b"%08X" % value


def pad4(stream, written):
    """newc aligns headers and file data to four bytes."""
    remainder = written % 4
    if remainder:
        stream.write(b"\0" * (4 - remainder))
        return 4 - remainder
    return 0


def write_entry(stream, name, st, data, ino):
    """Emit one newc header plus its name and data."""
    header = (
        MAGIC
        + field(ino)
        + field(st.st_mode)
        # Everything in the initramfs is owned by root. The build runs in a
        # user namespace where only uid 0 is mapped, so the staged tree's
        # ownership is not meaningful anyway -- stating 0 is both correct and
        # reproducible.
        + field(0)
        + field(0)
        + field(st.st_nlink)
        # mtime 0: a reproducible archive. The initrd is measured into a TPM
        # PCR by systemd-stub, so a timestamp would change the measurement on
        # every rebuild for no reason.
        + field(0)
        + field(len(data))
        + field(0) * 4          # devmajor, devminor, rdevmajor, rdevminor
        + field(len(name) + 1)  # namesize, including the NUL
        + field(0)              # check, unused for newc
    )
    stream.write(header)
    written = len(header)

    encoded = name.encode() + b"\0"
    stream.write(encoded)
    written += len(encoded)
    written += pad4(stream, written)

    if data:
        stream.write(data)
        pad4(stream, len(data))


def collect(root):
    """Every path under `root`, directories before their contents.

    Sorted so the archive is byte-identical across runs; the initrd ends up
    inside a UKI that gets measured, so reproducibility is not cosmetic.
    """
    entries = []
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames.sort()
        filenames.sort()
        here = Path(dirpath)
        if here != root:
            entries.append(here)
        for name in filenames:
            entries.append(here / name)
    return entries


def main():
    parser = argparse.ArgumentParser(
        description=__doc__.splitlines()[0] if __doc__ else None
    )
    parser.add_argument("root", help="directory to archive")
    parser.add_argument("output", help="cpio file to write")
    args = parser.parse_args()

    root = Path(args.root).resolve()
    if not root.is_dir():
        sys.exit(f"mkcpio: {root} is not a directory")

    count = 0
    with open(args.output, "wb") as stream:
        for ino, path in enumerate(collect(root), start=1):
            st = path.lstat()
            name = str(path.relative_to(root))

            if stat.S_ISLNK(st.st_mode):
                data = os.readlink(path).encode()
            elif stat.S_ISREG(st.st_mode):
                data = path.read_bytes()
            elif stat.S_ISDIR(st.st_mode):
                data = b""
            else:
                # Device nodes, sockets and FIFOs. The initrd needs none:
                # systemd mounts devtmpfs itself as its very first action.
                print(f"mkcpio: skipping non-regular {name}", file=sys.stderr)
                continue

            write_entry(stream, name, st, data, ino)
            count += 1

        # The trailer is a zero-length entry with a fixed name and nlink 1.
        class Trailer:
            st_mode = 0
            st_nlink = 1

        write_entry(stream, TRAILER, Trailer(), b"", 0)
        # The archive itself is padded to a 512-byte boundary; the kernel's
        # unpacker tolerates the absence, but every other producer does it.
        # Spelled as a negative modulo so an already-aligned archive gets zero
        # padding rather than a whole extra block.
        stream.write(b"\0" * ((-stream.tell()) % 512))

    size = Path(args.output).stat().st_size
    print(f"mkcpio: {count} entries, {size} bytes -> {args.output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
