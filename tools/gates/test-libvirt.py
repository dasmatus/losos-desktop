#!/usr/bin/env python3
"""Check the libvirt domain against the claims it is supposed to make.

`tools/libvirt-domain` exists so that looking at the desktop is as easy as
`virsh define`. The risk it carries is subtler than a malformed file, which
libvirt would reject on the spot: a domain that starts, shows a desktop, and
proves nothing, because the host handed the guest something the image is
supposed to have provided for itself.

That is exactly the mistake `tools/vm-test disk` was written to avoid -- its
sibling `vm-test boot` stages an ESP with QEMU, so it says nothing about the
partition table -- and it is worth asserting here rather than rediscovering.

So the invariants below are mostly negative. No kernel, no initrd, no kernel
command line: firmware has to find the ESP in the image's own GPT, and
systemd-gpt-auto-generator has to find the root filesystem by its type.

No libvirt is needed to run this, which is the point: `./do check` has to pass
on a machine with nothing but Python.
"""

import sys
from importlib.machinery import SourceFileLoader
from pathlib import Path
from xml.etree import ElementTree

REPO = Path(__file__).resolve().parent.parent.parent
domain_tool = SourceFileLoader(
    "libvirt_domain", str(REPO / "tools" / "libvirt-domain")
).load_module()

# Elements that would hand the guest something the image must supply itself.
# `<os><kernel>` is the direct boot path: firmware is bypassed, the ESP is never
# read, and an image whose partition table is wrong boots perfectly.
FORBIDDEN = ("kernel", "initrd", "cmdline", "boot")


def check(arch, failures):
    domain = domain_tool.build(
        disk=Path("/nonexistent/losos.qcow2"),
        arch=arch,
        name="losos",
        memory_mib=4096,
        vcpus=4,
        accel=True,
        credentials=["set-hostname=probe"],
    )
    # Round-trip through serialisation: an ElementTree that is fine in memory
    # can still be a document libvirt will not parse.
    domain = ElementTree.fromstring(ElementTree.tostring(domain, encoding="unicode"))

    def fail(message):
        failures.append(f"{arch}: {message}")

    os_element = domain.find("os")
    for tag in FORBIDDEN:
        if os_element.find(tag) is not None:
            fail(f"<os> carries <{tag}>, so the guest is not booting its own disk")

    if os_element.get("firmware") != "efi":
        fail("the domain is not a UEFI machine; this image has no MBR to fall back to")
    if os_element.find("nvram") is None:
        fail("no <nvram>, so every boot is a first boot and no boot entry survives")

    secure = os_element.find("firmware/feature[@name='secure-boot']")
    if secure is None or secure.get("enabled") != "no":
        fail("Secure Boot is not explicitly disabled; the UKI is signed by nobody")

    disks = domain.findall("devices/disk")
    if len(disks) != 1:
        fail(f"{len(disks)} disks, expected exactly the image under test")
    else:
        driver = disks[0].find("driver")
        if driver.get("type") != "qcow2":
            fail("the disk is not attached as qcow2, so the guest sees its header")

    if domain.find("devices/rng") is None:
        fail("no virtio-rng: the desktop's first boot blocks on entropy")

    console = domain.find("devices/console/target")
    expected = domain_tool.MACHINES[arch]["console"]
    if console is None or console.get("type") != expected:
        # On aarch64 a serial console lands on a port the machine does not
        # have, and the guest's output is lost with no error anywhere.
        fail(f"console target is {console.get('type') if console else None}, expected {expected}")

    entry = domain.find("sysinfo/entry")
    if entry is None or not entry.get("name", "").startswith("opt/io.systemd.credentials/"):
        fail("a --credential did not become a systemd credential in fw_cfg")

    print(f"  {arch:9} UEFI, own disk, {len(domain.findall('devices/*'))} devices")


def main():
    failures = []
    for arch in sorted(domain_tool.MACHINES):
        check(arch, failures)

    if failures:
        print("test-libvirt: FAILED", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1

    print("test-libvirt: the domain boots the image the way firmware would")
    return 0


if __name__ == "__main__":
    sys.exit(main())
