#!/usr/bin/env python3
"""Assemble a Unified Kernel Image by appending PE sections to systemd-stub.

A UKI is systemd's `linuxx64.efi.stub` with the kernel, the initrd, the kernel
command line, os-release and an SBAT record added as named PE/COFF sections.
The firmware loads one EFI binary; the stub finds its own sections, measures
them into TPM PCR 11, and boots the kernel inside itself. That is what makes
the kernel, its initrd and its command line one signable, measurable unit --
the property this OS's whole boot story rests on.

Two tools already do this and neither is usable here:

  * `ukify`, shipped by systemd, needs python-pefile. That is a third-party
    module, it is not in the staged tree, and installing it would mean `pip`
    in a build step -- which grants the whole layer Network (C8).
  * `objcopy --add-section` is fingerprinted and would work, but it recomputes
    almost nothing: section virtual addresses have to be assigned by hand
    anyway, so the fiddly part is not avoided, only moved into flags.

So the PE surgery is done here, with `struct` and nothing else. It is the same
job `ukify` does, minus the signing, which belongs to whoever holds the key.

Reference: PE/COFF specification, section table; systemd's src/fundamental/
uki.h for the section names the stub looks for.
"""

import argparse
import struct
import sys
from pathlib import Path

# The sections systemd-stub reads, in the order ukify writes them. Order is not
# load-bearing for the stub, but a stable order keeps the output reproducible.
SECTIONS = ["osrel", "cmdline", "uname", "sbat", "linux", "initrd"]

PE_SIGNATURE_OFFSET = 0x3C
SECTION_HEADER_SIZE = 40
CHARACTERISTICS_RODATA = 0x40000040  # initialised data, read-only


def align(value, boundary):
    return (value + boundary - 1) // boundary * boundary


def read_pe(data):
    """Locate the COFF header and pull out what we need to extend it."""
    pe_offset = struct.unpack_from("<I", data, PE_SIGNATURE_OFFSET)[0]
    if data[pe_offset : pe_offset + 4] != b"PE\0\0":
        sys.exit("mkuki: stub is not a PE binary")

    coff = pe_offset + 4
    sections, _, _, _, optional_size, _ = struct.unpack_from("<HIIIHH", data, coff + 2)
    optional = coff + 20
    magic = struct.unpack_from("<H", data, optional)[0]
    if magic != 0x20B:
        sys.exit("mkuki: stub is not PE32+ (x86-64)")

    section_alignment = struct.unpack_from("<I", data, optional + 32)[0]
    file_alignment = struct.unpack_from("<I", data, optional + 36)[0]
    return {
        "pe_offset": pe_offset,
        "coff": coff,
        "optional": optional,
        "optional_size": optional_size,
        "sections": sections,
        "section_table": optional + optional_size,
        "section_alignment": section_alignment,
        "file_alignment": file_alignment,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--stub", required=True, help="linuxx64.efi.stub")
    parser.add_argument("--linux", required=True, help="the kernel image")
    parser.add_argument("--initrd", required=True)
    parser.add_argument("--osrel", required=True)
    parser.add_argument("--cmdline", required=True)
    parser.add_argument("--uname", required=True, help="kernel version string")
    parser.add_argument("--sbat", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()

    stub = Path(args.stub).read_bytes()
    pe = read_pe(stub)

    payloads = {
        "osrel": Path(args.osrel).read_bytes(),
        "cmdline": Path(args.cmdline).read_bytes().strip() + b"\0",
        "uname": args.uname.encode() + b"\0",
        "sbat": Path(args.sbat).read_bytes(),
        "linux": Path(args.linux).read_bytes(),
        "initrd": Path(args.initrd).read_bytes(),
    }

    # The new sections start after everything the stub already occupies, in
    # both address space and file offset.
    next_rva = 0
    next_offset = 0
    for index in range(pe["sections"]):
        header = pe["section_table"] + index * SECTION_HEADER_SIZE
        virtual_size, virtual_address, raw_size, raw_offset = struct.unpack_from(
            "<IIII", stub, header + 8
        )
        next_rva = max(next_rva, virtual_address + virtual_size)
        next_offset = max(next_offset, raw_offset + raw_size)

    next_rva = align(next_rva, pe["section_alignment"])
    next_offset = align(next_offset, pe["file_alignment"])

    # There must be room between the end of the existing section table and the
    # first section's data, or the new headers would overwrite payload.
    table_end = pe["section_table"] + pe["sections"] * SECTION_HEADER_SIZE
    needed = table_end + len(SECTIONS) * SECTION_HEADER_SIZE
    first_raw = min(
        struct.unpack_from("<I", stub, pe["section_table"] + i * SECTION_HEADER_SIZE + 20)[0]
        for i in range(pe["sections"])
    )
    if needed > first_raw:
        sys.exit(
            f"mkuki: no room for {len(SECTIONS)} more section headers "
            f"(need {needed}, first section data at {first_raw}). "
            "The stub's header padding is too small; ukify hits this too."
        )

    output = bytearray(stub)
    output.extend(b"\0" * (next_offset - len(output)))

    headers = bytearray()
    for name in SECTIONS:
        body = payloads[name]
        raw_size = align(len(body), pe["file_alignment"])

        headers += struct.pack(
            "<8sIIII12xI",
            # The stub matches on these names; ".linux" and ".initrd" are the
            # ones that make it a UKI rather than a plain EFI binary.
            ("." + name).encode(),
            len(body),        # VirtualSize
            next_rva,         # VirtualAddress
            raw_size,         # SizeOfRawData
            next_offset,      # PointerToRawData
            CHARACTERISTICS_RODATA,
        )

        output.extend(body)
        output.extend(b"\0" * (raw_size - len(body)))

        next_rva = align(next_rva + len(body), pe["section_alignment"])
        next_offset += raw_size

    output[table_end : table_end + len(headers)] = headers

    # Fix up the COFF and optional headers to account for the new sections.
    struct.pack_into("<H", output, pe["coff"] + 2, pe["sections"] + len(SECTIONS))
    struct.pack_into("<I", output, pe["optional"] + 56, next_rva)  # SizeOfImage

    Path(args.output).write_bytes(output)
    print(
        f"mkuki: {len(SECTIONS)} sections appended, "
        f"{len(output)} bytes -> {args.output}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
