#!/usr/bin/env python3
"""Rewrite a staged rootfs into a `/usr`-merged layout."""

import argparse
import os
import shutil
import sys
from collections import deque
from pathlib import Path

LINKS = {
    "bin": "usr/bin",
    "sbin": "usr/sbin",
    "lib": "usr/lib",
    "lib32": "usr/lib32",
    "lib64": "usr/lib64",
}
# Match the symlink-resolution limit common kernels and libc implementations use,
# so hostile chains terminate predictably without constraining legitimate trees.
MAX_SYMLINK_HOPS = 40


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


def resolve_source_path(root, relative):
    current = root
    pending = deque(Path(relative).parts)
    seen = set()
    hops = 0
    followed_symlink = False

    while pending:
        part = pending.popleft()
        if part in ("", "."):
            continue
        if part == "..":
            current = current.parent
            try:
                current.relative_to(root)
            except ValueError:
                return None
            continue

        current = current / part
        while current.is_symlink():
            state = (current, tuple(pending))
            if state in seen or hops >= MAX_SYMLINK_HOPS:
                return None
            seen.add(state)
            hops += 1
            followed_symlink = True
            target = current.readlink()
            if target.is_absolute():
                current = root
                pending.extendleft(reversed(Path(str(target).lstrip("/")).parts))
            else:
                current = current.parent
                pending.extendleft(reversed(target.parts))

        try:
            current.relative_to(root)
        except ValueError:
            return None
        if not current.exists() and not pending:
            if followed_symlink:
                return None
            return current

    return current


def check_source_path(root, relative):
    resolved = resolve_source_path(root, relative)
    if resolved is None:
        return fail(f"/{relative} resolves outside the staged root")
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
    if target.is_symlink() or (target.exists() and not target.is_dir()):
        return fail(f"{target} already exists and is not a directory")
    return 0


def check_merge_dir(root, source_name, target_name):
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
    return 0


def merge_dir(root, source_name, target_name):
    source = root / source_name
    target = root / target_name

    if source.is_symlink():
        return 0

    target.mkdir(parents=True, exist_ok=True)
    if source.is_dir():
        for child in list(source.iterdir()):
            status = move_entry(child, target / child.name)
            if status:
                return status
        source.rmdir()

    source.symlink_to(f"/{target_name}")
    return 0


def os_release(root):
    status = check_source_path(root, "usr/lib/os-release")
    if status:
        return status
    status = check_os_release_parent(root)
    if status:
        return status
    source = root / "usr/lib/os-release"
    target = root / "etc/os-release"
    if target.is_symlink():
        if target_path(root, target) != source:
            return fail(f"/etc/os-release already links to {target.readlink()}, not ../usr/lib/os-release")
        return 0
    if target.exists():
        return fail("/etc/os-release exists and is not a symlink")
    target.parent.mkdir(parents=True, exist_ok=True)
    target.symlink_to("../usr/lib/os-release")
    return 0


def check_os_release(root):
    status = check_os_release_parent(root)
    if status:
        return status
    source = root / "usr/lib/os-release"
    target = root / "etc/os-release"
    if target.is_symlink():
        if target_path(root, target) != source:
            return fail(f"/etc/os-release already links to {target.readlink()}, not ../usr/lib/os-release")
        return 0
    if target.exists():
        return fail("/etc/os-release exists and is not a symlink")
    return 0


def check_os_release_parent(root):
    parent = root / "etc"
    resolved_root = root.resolve(strict=False)
    if parent.is_symlink():
        return fail("/etc exists and is a symlink")
    if parent.exists() and not parent.is_dir():
        return fail("/etc exists and is not a directory")
    try:
        parent.resolve(strict=False).relative_to(resolved_root)
    except ValueError:
        return fail("/etc resolves outside the staged root")
    return 0


def preflight(root):
    for source_name, target_name in LINKS.items():
        status = check_merge_dir(root, source_name, target_name)
        if status:
            return status
    status = check_os_release(root)
    if status:
        return status
    return check_source_path(root, "usr/lib/os-release")


def main():
    parser = argparse.ArgumentParser(
        description=__doc__.splitlines()[0] if __doc__ else None
    )
    parser.add_argument("root", help="assembled root filesystem to rewrite")
    args = parser.parse_args()

    root = Path(args.root)
    status = preflight(root)
    if status:
        return status
    for source_name, target_name in LINKS.items():
        status = merge_dir(root, source_name, target_name)
        if status:
            return status

    return os_release(root)


if __name__ == "__main__":
    sys.exit(main())
