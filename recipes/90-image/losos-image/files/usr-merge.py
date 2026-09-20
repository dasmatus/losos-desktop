#!/usr/bin/env python3
"""Rewrite a staged rootfs into a `/usr`-merged layout."""

import argparse
import os
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


def target_path(root, link):
    target = link.readlink()
    if target.is_absolute():
        candidate = root / str(target).lstrip("/")
    else:
        candidate = Path(os.path.normpath(str(link.parent / target)))
    try:
        candidate.resolve(strict=False).relative_to(root.resolve(strict=False))
    except ValueError:
        return None
    return candidate


def check_source_path(root, relative):
    path = root / relative
    try:
        resolved = path.resolve(strict=False)
        resolved.relative_to(root.resolve())
    except ValueError:
        return fail(f"/{relative} resolves outside the staged root")

    try:
        path.lstat()
    except FileNotFoundError:
        return fail(f"/{relative} is missing")
    if not resolved.exists():
        return fail(f"/{relative} is missing")
    return 0


def move_entry(source, destination):
    if source.is_dir() and not source.is_symlink():
        if destination.exists():
            if destination.is_symlink() or not destination.is_dir():
                return fail(f"{destination} already exists and is not a directory")
            for child in list(source.iterdir()):
                status = move_entry(child, destination / child.name)
                if status:
                    return status
            source.rmdir()
            return 0
        shutil.move(str(source), destination)
        return 0

    if destination.exists() or destination.is_symlink():
        return fail(f"{destination} already exists")
    shutil.move(str(source), destination)
    return 0


def check_entry(source, destination):
    if source.is_dir() and not source.is_symlink():
        if destination.exists() or destination.is_symlink():
            if destination.is_symlink() or not destination.is_dir():
                return fail(f"{destination} already exists and is not a directory")
            for child in source.iterdir():
                status = check_entry(child, destination / child.name)
                if status:
                    return status
        return 0

    if destination.exists() or destination.is_symlink():
        return fail(f"{destination} already exists")
    return 0


def check_target_dir(target):
    if target.exists() and not target.is_dir():
        return fail(f"{target} already exists and is not a directory")
    if target.is_symlink():
        return fail(f"{target} already exists and is not a directory")
    return 0


def merge_dir(root, source_name, target_name):
    source = root / source_name
    target = root / target_name

    if source.is_symlink():
        if target_path(root, source) != target:
            return fail(f"/{source_name} already links to {source.readlink()}, not {target_name}")
        return 0
    if source.exists() and not source.is_dir():
        return fail(f"/{source_name} exists but is not a directory")

    status = check_target_dir(target)
    if status:
        return status
    if source.is_dir():
        status = check_entry(source, target)
        if status:
            return status
    target.mkdir(parents=True, exist_ok=True)
    if source.is_dir():
        for child in list(source.iterdir()):
            status = move_entry(child, target / child.name)
            if status:
                return status
        source.rmdir()

    source.symlink_to(target_name)
    return 0


def os_release(root):
    source = root / "usr/lib/os-release"
    target = root / "etc/os-release"
    target.parent.mkdir(parents=True, exist_ok=True)
    if target.is_symlink():
        if target_path(root, target) != source:
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
    status = check_source_path(root, "usr/lib/os-release")
    if status:
        return status
    for source_name, target_name in LINKS.items():
        status = merge_dir(root, source_name, target_name)
        if status:
            return status

    return os_release(root)


if __name__ == "__main__":
    sys.exit(main())
