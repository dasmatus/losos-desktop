#!/usr/bin/env python3
"""Populate an initrd root from the staged OS tree.

systemd is the init in the initrd, not just on the real root: `-Dinitrd=true`
builds the initrd-side units, and systemd-stub hands PID 1 the same binary in
both phases. That is what lets the initrd run systemd-repart, systemd-veritysetup
and systemd-cryptsetup with the same configuration the booted system uses,
rather than a second, differently-configured copy of everything.

What goes in is a list, not a guess: files/initrd-manifest.txt. A closure
computed with ldd would be neater and is not available -- ldd runs the loader on
the target binary, and the staged tree's binaries are built against the build
host's libc (see docs/limits.md), so the answer would describe the build host
rather than the image.

Deliberately stdlib-only; it runs under the `python` fingerprint, which grants
no network.
"""

import argparse
import shutil
import sys
from pathlib import Path


def copy_entry(source_root, dest_root, relative, missing):
    src = source_root / relative
    if not src.exists() and not src.is_symlink():
        missing.append(relative)
        return 0

    dest = dest_root / relative
    dest.parent.mkdir(parents=True, exist_ok=True)

    if src.is_dir() and not src.is_symlink():
        shutil.copytree(src, dest, symlinks=True, dirs_exist_ok=True)
        return sum(1 for _ in dest.rglob("*"))

    # symlinks=True everywhere: the staged tree is full of them, and resolving
    # one here would silently double the size of the initrd.
    shutil.copy2(src, dest, follow_symlinks=False)
    return 1


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", required=True, help="the staged OS tree")
    parser.add_argument("--out", required=True, help="initrd root to populate")
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--kver", required=True)
    args = parser.parse_args()

    source = Path(args.root)
    dest = Path(args.out)
    dest.mkdir(parents=True, exist_ok=True)

    copied, missing = 0, []
    for line in Path(args.manifest).read_text().splitlines():
        line = line.split("#", 1)[0].strip()
        if not line:
            continue
        # @KVER@ lets the manifest name a modules directory without the
        # version being written down in two places.
        copied += copy_entry(source, dest, line.replace("@KVER@", args.kver), missing)

    # The directories systemd expects to exist before it mounts anything. It
    # creates most itself, but /init must be there for the kernel to exec.
    for directory in ("dev", "proc", "sys", "run", "tmp", "etc", "sysroot"):
        (dest / directory).mkdir(parents=True, exist_ok=True)

    # The kernel execs /init. systemd handles being PID 1 in an initrd by
    # checking for /etc/initrd-release, which is also how it knows to run the
    # initrd unit set rather than the host one.
    init = dest / "init"
    if not init.exists():
        init.symlink_to("usr/lib/systemd/systemd")
    (dest / "etc" / "initrd-release").write_text(
        (source / "usr" / "lib" / "os-release").read_text()
        if (source / "usr" / "lib" / "os-release").exists()
        else "ID=losos-desktop\n"
    )

    if missing:
        print(f"mkinitrd: {len(missing)} manifest entr(ies) absent from the tree:",
              file=sys.stderr)
        for entry in missing:
            print(f"  {entry}", file=sys.stderr)
        # Hard failure: an initrd missing systemd-repart boots to a system with
        # no root partition, and says nothing about why.
        return 1

    print(f"mkinitrd: {copied} path(s) -> {dest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
