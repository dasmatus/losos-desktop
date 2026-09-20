#!/usr/bin/env python3
"""Rewrite a staged rootfs into a `/usr`-merged layout."""

import argparse
import shutil
import sys
from pathlib import Path

LINKS = {
    "bin": "usr/bin",
    "sbin": "usr/sbin",
    "lib": "usr/lib",
    "lib32": "usr/lib32",
    "lib64": "usr/lib64",
}


def fail(message):
    print(f"usr-merge: {message}", file=sys.stderr)
    return 1


def merge_dir(root, source_name, target_name):
    source = root / source_name
    target = root / target_name

    if source.is_symlink():
        if source.readlink().as_posix() != target_name:
            return fail(f"/{source_name} already links to {source.readlink()}, not {target_name}")
        return 0
    if source.exists() and not source.is_dir():
        return fail(f"/{source_name} exists but is not a directory")

    target.mkdir(parents=True, exist_ok=True)
    if source.is_dir():
        for child in list(source.iterdir()):
            destination = target / child.name
            if destination.exists() or destination.is_symlink():
                return fail(f"/{target_name}/{child.name} already exists")
            shutil.move(str(child), destination)
        source.rmdir()

    source.symlink_to(target_name)
    return 0


def os_release(root):
    source = root / "usr/lib/os-release"
    target = root / "etc/os-release"
    if not source.exists():
        return 0

    target.parent.mkdir(parents=True, exist_ok=True)
    if target.is_symlink():
        if target.readlink().as_posix() != "../usr/lib/os-release":
            return fail(f"/etc/os-release already links to {target.readlink()}, not ../usr/lib/os-release")
        return 0
    if target.exists():
        return fail("/etc/os-release exists and is not a symlink")

    target.symlink_to("../usr/lib/os-release")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("root", help="assembled root filesystem to rewrite")
    args = parser.parse_args()

    root = Path(args.root)
    for source_name, target_name in LINKS.items():
        status = merge_dir(root, source_name, target_name)
        if status:
            return status

    return os_release(root)


if __name__ == "__main__":
    sys.exit(main())
