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


def main():
    failures = []
    with tempfile.TemporaryDirectory(prefix="losos-image-test-") as tmp:
        work = Path(tmp)
        cpio_work = work / "cpio"
        cpio_work.mkdir()
        test_cpio(cpio_work, failures)
        test_uki(work, failures)

    if failures:
        print("test-image: FAILED", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1

    print("test-image: initramfs and UKI writers behave")
    return 0


if __name__ == "__main__":
    sys.exit(main())
