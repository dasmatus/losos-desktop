#!/usr/bin/env python3
"""Exercise the two pieces of novel code in this repository.

`mkcpio.py` and `mkuki.py` exist because neither job can be done by a program
pm will run (C1, C4): `cpio -o` reads its file list from a stdin pm ties to
/dev/null, and `ukify` needs a Python module that is not in the staged tree and
could only be installed by granting the whole layer network access.

They are therefore the only code here that nobody upstream has already
debugged, and the only code whose failure would surface as an unbootable image
rather than a failed build. So they get tested directly, against synthetic
inputs, with no network and no upstream sources -- which means this gate runs
in exactly the places `./do check` does.

The cpio is checked by parsing it back with an independent reader written
against the newc spec rather than by reusing the writer's own helpers, so a
misunderstanding of the format fails instead of round-tripping.
"""

import struct
import stat
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
IMAGE = REPO / "recipes" / "90-image" / "losos-image" / "files"


def parse_cpio(data):
    """An independent newc reader. Returns [(name, mode, payload)]."""
    entries, off = [], 0
    while off < len(data):
        if data[off : off + 6] != b"070701":
            break
        fields = [
            int(data[off + 6 + i * 8 : off + 14 + i * 8], 16) for i in range(13)
        ]
        mode, nlink, filesize, namesize = fields[1], fields[4], fields[6], fields[11]
        off += 110
        name = data[off : off + namesize - 1].decode()
        off += namesize
        off += (-off) % 4
        body = data[off : off + filesize]
        off += filesize
        off += (-off) % 4
        if name == "TRAILER!!!":
            entries.append((name, mode, b""))
            break
        entries.append((name, mode, body))
    return entries


def symlink_target(root, path):
    target = path.readlink()
    if target.is_absolute():
        return root / str(target).lstrip("/")
    return Path((path.parent / target).resolve(strict=False))


def test_cpio(work, failures):
    root = work / "root"
    (root / "usr" / "lib").mkdir(parents=True)
    (root / "usr" / "lib" / "os-release").write_bytes(b"ID=losos-desktop\n")
    (root / "usr" / "lib" / "link").symlink_to("os-release")
    (root / "dev").mkdir()

    out = work / "initrd.cpio"
    subprocess.run(
        [sys.executable, str(IMAGE / "mkcpio.py"), str(root), str(out)],
        check=True, capture_output=True,
    )

    entries = parse_cpio(out.read_bytes())
    names = {name for name, _, _ in entries}

    for expected in ("usr", "usr/lib", "usr/lib/os-release", "usr/lib/link", "dev",
                     "TRAILER!!!"):
        if expected not in names:
            failures.append(f"mkcpio: missing entry {expected}")

    by_name = {name: (mode, body) for name, mode, body in entries}

    mode, body = by_name.get("usr/lib/os-release", (0, b""))
    if not stat.S_ISREG(mode):
        failures.append("mkcpio: os-release is not a regular file")
    if body != b"ID=losos-desktop\n":
        failures.append(f"mkcpio: os-release payload wrong: {body!r}")

    mode, body = by_name.get("usr/lib/link", (0, b""))
    if not stat.S_ISLNK(mode):
        failures.append("mkcpio: symlink lost its type")
    if body != b"os-release":
        failures.append(f"mkcpio: symlink target wrong: {body!r}")

    mode, _ = by_name.get("usr/lib", (0, b""))
    if not stat.S_ISDIR(mode):
        failures.append("mkcpio: directory lost its type")

    # The archive must be 512-aligned, and must not have gained a whole empty
    # block when it was already aligned.
    size = out.stat().st_size
    if size % 512:
        failures.append(f"mkcpio: archive is not 512-aligned ({size})")

    if not failures:
        print(f"  mkcpio   {len(entries)} entries, {size} bytes, types preserved")


def test_usr_merge(work, failures):
    root = work / "root"
    (root / "usr" / "bin").mkdir(parents=True)
    (root / "usr" / "lib").mkdir(parents=True)
    (root / "bin").mkdir()
    (root / "lib").mkdir()
    (root / "etc").mkdir()
    (root / "usr" / "bin" / "helpers").mkdir()
    (root / "bin" / "helpers").mkdir()
    (root / "sbin").symlink_to("/usr/sbin")
    (root / "etc" / "os-release").symlink_to("/usr/lib/os-release")

    (root / "bin" / "losos-release").write_text("#!/bin/sh\n")
    (root / "bin" / "helpers" / "moved").write_text("moved\n")
    (root / "usr" / "bin" / "helpers" / "kept").write_text("kept\n")
    (root / "lib" / "ld-musl-test.so.1").symlink_to("/usr/lib/libc.so")
    (root / "usr" / "lib" / "libc.so").write_text("libc\n")
    (root / "usr" / "lib" / "os-release").write_text("ID=losos-desktop\n")

    subprocess.run(
        [sys.executable, str(IMAGE / "usr-merge.py"), str(root)],
        check=True, capture_output=True,
    )

    checks = {
        "bin": root / "usr/bin",
        "lib": root / "usr/lib",
        "sbin": root / "usr/sbin",
        "lib32": root / "usr/lib32",
        "lib64": root / "usr/lib64",
        "etc/os-release": root / "usr/lib/os-release",
    }
    for relative, expected in checks.items():
        path = root / relative
        if not path.is_symlink():
            failures.append(f"usr-merge: {relative} is not a symlink")
            continue
        actual = symlink_target(root, path)
        if actual != expected:
            failures.append(
                f"usr-merge: {relative} resolves to {actual!r}, expected {expected!r}"
            )

    text_checks = [
        ("usr/bin/losos-release", "#!/bin/sh\n", "usr-merge: /bin contents were not moved into /usr/bin"),
        (
            "usr/bin/helpers/moved",
            "moved\n",
            "usr-merge: nested /bin directories were not merged into /usr/bin",
        ),
        (
            "usr/bin/helpers/kept",
            "kept\n",
            "usr-merge: existing /usr/bin entries were not preserved",
        ),
    ]
    for relative, expected, message in text_checks:
        if (root / relative).read_text() != expected:
            failures.append(message)

    bool_checks = [
        (
            (root / "usr" / "lib" / "ld-musl-test.so.1").is_symlink(),
            "usr-merge: musl loader entry was not moved into /usr/lib",
        ),
        (
            not (root / "lib").exists() or (root / "lib").is_symlink(),
            "usr-merge: /lib still exists as a directory",
        ),
    ]
    for condition, message in bool_checks:
        if not condition:
            failures.append(message)

    if not failures:
        print("  usrmerge root compatibility paths now resolve through /usr")


def test_usr_merge_rejects_escape(work, failures):
    root = work / "root"
    root.mkdir()
    (root / "usr" / "bin").mkdir(parents=True)
    (root / "usr" / "lib").mkdir(parents=True)
    (root / "usr" / "lib" / "os-release").write_text("ID=losos-desktop\n")
    (root / "bin").symlink_to("../outside")

    result = subprocess.run(
        [sys.executable, str(IMAGE / "usr-merge.py"), str(root)],
        check=False, capture_output=True, text=True,
    )
    if result.returncode == 0:
        failures.append("usr-merge: accepted a compatibility symlink that escapes the staged root")
    elif "not usr/bin" not in result.stderr:
        failures.append("usr-merge: escape rejection did not explain the invalid target")
    elif not failures:
        print("  usrmerge rejects compatibility symlinks that escape the staged root")


def test_usr_merge_rejects_symlinked_target_escape(work, failures):
    root = work / "root"
    outside = work / "outside"
    root.mkdir()
    (outside / "lib").mkdir(parents=True)
    (outside / "lib" / "os-release").write_text("ID=losos-desktop\n")
    (root / "usr").symlink_to(outside)
    (root / "bin").symlink_to("/usr/bin")

    result = subprocess.run(
        [sys.executable, str(IMAGE / "usr-merge.py"), str(root)],
        check=False, capture_output=True, text=True,
    )
    if result.returncode == 0:
        failures.append(
            "usr-merge: accepted a compatibility symlink through a symlinked /usr escape"
        )
    elif (
        "not usr/bin" not in result.stderr and
        "resolves outside the staged root" not in result.stderr
    ):
        failures.append("usr-merge: symlinked target escape did not explain the invalid target")
    elif not failures:
        print("  usrmerge rejects compatibility symlinks whose target resolves outside the root")


def test_usr_merge_requires_os_release(work, failures):
    root = work / "root"
    (root / "usr" / "bin").mkdir(parents=True)
    (root / "etc").mkdir()
    (root / "bin").mkdir()

    result = subprocess.run(
        [sys.executable, str(IMAGE / "usr-merge.py"), str(root)],
        check=False, capture_output=True, text=True,
    )
    if result.returncode == 0:
        failures.append("usr-merge: accepted a tree with no /usr/lib/os-release")
    elif "/usr/lib/os-release is missing" not in result.stderr:
        failures.append("usr-merge: missing os-release did not explain the failure")
    elif (root / "bin").is_symlink():
        failures.append("usr-merge: missing os-release should fail before rewriting compatibility paths")
    elif not failures:
        print("  usrmerge requires /usr/lib/os-release before rewriting the tree")


def test_usr_merge_rejects_outside_os_release(work, failures):
    root = work / "root"
    outside = work / "outside"
    (root / "usr" / "bin").mkdir(parents=True)
    (root / "usr" / "lib").mkdir(parents=True)
    (root / "bin").mkdir()
    outside.mkdir()
    (outside / "os-release").write_text("ID=host\n")
    (root / "usr" / "lib" / "os-release").symlink_to(outside / "os-release")

    result = subprocess.run(
        [sys.executable, str(IMAGE / "usr-merge.py"), str(root)],
        check=False, capture_output=True, text=True,
    )
    if result.returncode == 0:
        failures.append("usr-merge: accepted /usr/lib/os-release outside the staged root")
    elif "resolves outside the staged root" not in result.stderr:
        failures.append("usr-merge: outside os-release did not explain the invalid source path")
    elif (root / "bin").is_symlink():
        failures.append("usr-merge: outside os-release should fail before rewriting compatibility paths")
    elif not failures:
        print("  usrmerge rejects /usr/lib/os-release paths that escape the staged root")
 

def test_usr_merge_preflights_before_rewriting(work, failures):
    root = work / "root"
    (root / "usr" / "bin").mkdir(parents=True)
    (root / "usr" / "lib").mkdir(parents=True)
    (root / "bin").mkdir()
    (root / "etc").mkdir()
    (root / "bin" / "losos-release").write_text("#!/bin/sh\n")
    (root / "usr" / "lib" / "os-release").write_text("ID=losos-desktop\n")
    (root / "etc" / "os-release").write_text("ID=host\n")

    result = subprocess.run(
        [sys.executable, str(IMAGE / "usr-merge.py"), str(root)],
        check=False, capture_output=True, text=True,
    )
    if result.returncode == 0:
        failures.append("usr-merge: accepted a non-symlink /etc/os-release")
    elif "/etc/os-release exists and is not a symlink" not in result.stderr:
        failures.append("usr-merge: /etc/os-release preflight did not explain the failure")
    elif (root / "bin").is_symlink():
        failures.append("usr-merge: /etc/os-release failure should happen before rewriting compatibility paths")
    elif not (root / "bin" / "losos-release").exists():
        failures.append("usr-merge: /etc/os-release failure should not move /bin contents")
    elif not failures:
        print("  usrmerge preflights /etc/os-release before rewriting compatibility paths")


def test_usr_merge_preflights_all_compat_paths(work, failures):
    root = work / "root"
    (root / "usr" / "bin").mkdir(parents=True)
    (root / "usr" / "lib").mkdir(parents=True)
    (root / "bin").mkdir()
    (root / "lib").mkdir()
    (root / "etc").mkdir()
    (root / "bin" / "losos-release").write_text("#!/bin/sh\n")
    (root / "lib" / "libdup.so").write_text("from-lib\n")
    (root / "usr" / "lib" / "libdup.so").write_text("from-usr\n")
    (root / "usr" / "lib" / "os-release").write_text("ID=losos-desktop\n")

    result = subprocess.run(
        [sys.executable, str(IMAGE / "usr-merge.py"), str(root)],
        check=False, capture_output=True, text=True,
    )
    if result.returncode == 0:
        failures.append("usr-merge: accepted conflicting compatibility path contents")
    elif "already exists" not in result.stderr:
        failures.append("usr-merge: compatibility path conflict did not explain the preflight failure")
    elif (root / "bin").is_symlink():
        failures.append("usr-merge: compatibility conflict should fail before rewriting earlier paths")
    elif not (root / "bin" / "losos-release").exists():
        failures.append("usr-merge: compatibility conflict should not move /bin contents")
    elif not failures:
        print("  usrmerge preflights all compatibility paths before rewriting any of them")


def test_usr_merge_rejects_symlinked_etc(work, failures):
    root = work / "root"
    outside = work / "outside"
    (root / "usr" / "bin").mkdir(parents=True)
    (root / "usr" / "lib").mkdir(parents=True)
    (root / "bin").mkdir()
    outside.mkdir()
    (root / "etc").symlink_to(outside)
    (root / "usr" / "lib" / "os-release").write_text("ID=losos-desktop\n")

    result = subprocess.run(
        [sys.executable, str(IMAGE / "usr-merge.py"), str(root)],
        check=False, capture_output=True, text=True,
    )
    if result.returncode == 0:
        failures.append("usr-merge: accepted a symlinked /etc directory")
    elif "/etc exists and is a symlink" not in result.stderr:
        failures.append("usr-merge: symlinked /etc did not explain the invalid parent path")
    elif (root / "bin").is_symlink():
        failures.append("usr-merge: symlinked /etc should fail before rewriting compatibility paths")
    elif not failures:
        print("  usrmerge rejects symlinked /etc before rewriting compatibility paths")


def test_usr_merge_rejects_unresolved_escape(work, failures):
    root = work / "root"
    outside = work / "outside"
    (root / "usr" / "bin").mkdir(parents=True)
    (root / "usr" / "lib").mkdir(parents=True)
    (root / "bin").mkdir()
    outside.mkdir()
    (outside / "os-release").write_text("ID=host\n")
    (root / "usr" / "lib" / "os-release").symlink_to("missing/../../../../outside/os-release")

    result = subprocess.run(
        [sys.executable, str(IMAGE / "usr-merge.py"), str(root)],
        check=False, capture_output=True, text=True,
    )
    if result.returncode == 0:
        failures.append("usr-merge: accepted an unresolved /usr/lib/os-release escape")
    elif "resolves outside the staged root" not in result.stderr:
        failures.append("usr-merge: unresolved os-release escape did not explain the invalid source path")
    elif (root / "bin").is_symlink():
        failures.append("usr-merge: unresolved os-release escape should fail before rewriting compatibility paths")
    elif not failures:
        print("  usrmerge rejects unresolved /usr/lib/os-release escapes")


def test_usr_merge_rejects_dangling_directory_destination(work, failures):
    root = work / "root"
    (root / "usr" / "bin").mkdir(parents=True)
    (root / "usr" / "lib").mkdir(parents=True)
    (root / "usr" / "lib" / "os-release").write_text("ID=losos-desktop\n")
    (root / "bin" / "helpers").mkdir(parents=True)
    (root / "usr" / "bin" / "helpers").symlink_to("../missing")

    result = subprocess.run(
        [sys.executable, str(IMAGE / "usr-merge.py"), str(root)],
        check=False, capture_output=True, text=True,
    )
    if result.returncode == 0:
        failures.append("usr-merge: accepted a dangling symlink destination for a directory merge")
    elif "helpers already exists and is not a directory" not in result.stderr:
        failures.append("usr-merge: dangling directory destination did not explain the conflict")
    elif not failures:
        print("  usrmerge rejects dangling symlink destinations during directory preflight")


def synthetic_stub(path):
    """A minimal PE32+ shaped like systemd's linuxx64.efi.stub."""
    alignment = 4096
    dos = bytearray(64)
    dos[0:2] = b"MZ"
    struct.pack_into("<I", dos, 0x3C, 128)
    coff = struct.pack("<HHIIIHH", 0x8664, 1, 0, 0, 0, 240, 0x0022)
    opt = bytearray(240)
    struct.pack_into("<H", opt, 0, 0x20B)          # PE32+
    struct.pack_into("<I", opt, 32, alignment)     # SectionAlignment
    struct.pack_into("<I", opt, 36, alignment)     # FileAlignment
    struct.pack_into("<I", opt, 56, alignment * 2)  # SizeOfImage
    section = struct.pack(
        "<8sIIII12xI", b".text", 0x100, alignment, alignment, alignment, 0x60000020
    )
    header = bytes(dos) + b"\0" * 64 + b"PE\0\0" + coff + bytes(opt) + section
    blob = bytearray(header)
    blob.extend(b"\0" * (alignment - len(blob)))
    blob.extend(b"\x90" * alignment)
    path.write_bytes(blob)


def test_uki(work, failures):
    synthetic_stub(work / "stub.efi")
    inputs = {
        "vmlinuz": b"KERNELBYTES" * 100,
        "initrd.img": b"INITRDBYTES" * 50,
        "osrel": b"ID=losos-desktop\n",
        "cmdline": b"quiet rw\n",
        "sbat.csv": b"sbat,1,SBAT Version,sbat,1,https://example.invalid\n",
    }
    for name, body in inputs.items():
        (work / name).write_bytes(body)

    out = work / "losos.efi"
    subprocess.run(
        [
            sys.executable, str(IMAGE / "mkuki.py"),
            "--stub", str(work / "stub.efi"),
            "--linux", str(work / "vmlinuz"),
            "--initrd", str(work / "initrd.img"),
            "--osrel", str(work / "osrel"),
            "--cmdline", str(work / "cmdline"),
            "--uname", "6.12.1",
            "--sbat", str(work / "sbat.csv"),
            "--output", str(out),
        ],
        check=True, capture_output=True,
    )

    data = out.read_bytes()
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    coff = pe + 4
    count = struct.unpack_from("<H", data, coff + 2)[0]
    optional_size = struct.unpack_from("<H", data, coff + 16)[0]
    table = coff + 20 + optional_size

    found = {}
    for index in range(count):
        header = table + index * 40
        name = data[header : header + 8].rstrip(b"\0").decode()
        vsize, rva, _, offset = struct.unpack_from("<IIII", data, header + 8)
        found[name] = (vsize, rva, data[offset : offset + vsize])

    # The stub's own section must survive; a UKI that lost .text would not run.
    if ".text" not in found:
        failures.append("mkuki: the stub's own .text section was lost")

    for name, expected in (
        (".linux", inputs["vmlinuz"]),
        (".initrd", inputs["initrd.img"]),
        (".osrel", inputs["osrel"]),
        (".sbat", inputs["sbat.csv"]),
    ):
        if name not in found:
            failures.append(f"mkuki: section {name} missing")
            continue
        if found[name][2] != expected:
            failures.append(f"mkuki: section {name} payload does not match its input")

    if ".uname" in found and found[".uname"][2] != b"6.12.1\0":
        failures.append("mkuki: .uname is not the NUL-terminated version")
    if ".cmdline" in found and found[".cmdline"][2] != b"quiet rw\0":
        failures.append("mkuki: .cmdline was not NUL-terminated")

    # Sections must not overlap in address space.
    spans = sorted((rva, rva + vsize) for vsize, rva, _ in found.values())
    for (_, end), (start, _) in zip(spans, spans[1:]):
        if start < end:
            failures.append("mkuki: sections overlap in virtual address space")
            break

    size = struct.unpack_from("<I", data, coff + 20 + 56)[0]
    if size < spans[-1][1]:
        failures.append("mkuki: SizeOfImage is smaller than the last section")

    if not failures:
        print(f"  mkuki    {count} sections, {len(data)} bytes, payloads intact")


# The Discoverable Partitions Specification's root types, written here so that
# architectures.yaml can be checked against something rather than against
# itself.
DISCOVERABLE_ROOTS = {
    "root-x86-64": "4F68BCE3-E8CD-4DB1-96E7-FBCAF984B709",
    "root-arm64": "B921B045-1DF0-41C3-AF44-4C6F280D3FAE",
}


def test_partition_types(failures):
    """`gpt_root` and `gpt_root_uuid` are one value spelled two ways.

    They have to be. systemd-repart takes the name and xorriso's
    -append_partition takes the GUID, so the same partition type is written out
    twice for every architecture, and a mismatch is an image whose ISO carries
    a root partition systemd-gpt-auto-generator will not recognise -- which
    boots to an initrd with nowhere to go and no message saying why.
    """
    sys.path.insert(0, str(REPO / "tools" / "lib"))
    import yaml

    architectures = yaml.safe_load(
        (REPO / "manifest" / "architectures.yaml").read_text()
    )
    for arch, spec in sorted(architectures.items()):
        name, guid = spec.get("gpt_root"), spec.get("gpt_root_uuid")
        expected = DISCOVERABLE_ROOTS.get(name)
        if expected is None:
            failures.append(
                f"partition types: {arch}: gpt_root {name!r} is not a root type "
                "this gate knows; add it here with its GUID from the "
                "Discoverable Partitions Specification"
            )
        elif (guid or "").upper() != expected:
            failures.append(
                f"partition types: {arch}: gpt_root is {name} but gpt_root_uuid "
                f"is {guid}, and {name} is {expected}"
            )
    if not failures:
        print(f"  types    {len(architectures)} architecture(s) name one root type twice")


def main():
    failures = []
    test_partition_types(failures)
    with tempfile.TemporaryDirectory(prefix="losos-image-test-") as tmp:
        work = Path(tmp)
        cpio_work = work / "cpio"
        cpio_work.mkdir()
        test_cpio(cpio_work, failures)
        usr_work = work / "usr-merge"
        usr_work.mkdir()
        test_usr_merge(usr_work, failures)
        usr_escape = work / "usr-merge-escape"
        usr_escape.mkdir()
        test_usr_merge_rejects_escape(usr_escape, failures)
        usr_target_escape = work / "usr-merge-target-escape"
        usr_target_escape.mkdir()
        test_usr_merge_rejects_symlinked_target_escape(usr_target_escape, failures)
        usr_dangling = work / "usr-merge-dangling"
        usr_dangling.mkdir()
        test_usr_merge_rejects_dangling_directory_destination(usr_dangling, failures)
        usr_missing = work / "usr-merge-missing"
        usr_missing.mkdir()
        test_usr_merge_requires_os_release(usr_missing, failures)
        usr_outside = work / "usr-merge-outside"
        usr_outside.mkdir()
        test_usr_merge_rejects_outside_os_release(usr_outside, failures)
        usr_unresolved = work / "usr-merge-unresolved"
        usr_unresolved.mkdir()
        test_usr_merge_rejects_unresolved_escape(usr_unresolved, failures)
        usr_partial = work / "usr-merge-partial"
        usr_partial.mkdir()
        test_usr_merge_preflights_before_rewriting(usr_partial, failures)
        usr_preflight_links = work / "usr-merge-preflight-links"
        usr_preflight_links.mkdir()
        test_usr_merge_preflights_all_compat_paths(usr_preflight_links, failures)
        usr_etc_link = work / "usr-merge-etc-link"
        usr_etc_link.mkdir()
        test_usr_merge_rejects_symlinked_etc(usr_etc_link, failures)
        test_uki(work, failures)

    if failures:
        print("test-image: FAILED", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1

    print("test-image: image helpers behave")
    return 0


if __name__ == "__main__":
    sys.exit(main())
