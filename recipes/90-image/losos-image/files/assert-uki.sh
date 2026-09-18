#!/bin/sh
# Check that the UKI is a PE binary carrying the sections systemd-stub needs.
#
# Nothing here can boot the image, so this is the strongest statement the build
# can make about it: the file is a PE, and the section names the stub looks for
# are present. A UKI missing .linux is not a kernel image, it is a 30 MB EFI
# application that exits immediately -- and the firmware will say nothing useful
# about why.
#
# Usage: sh assert-uki.sh <uki>

set -eu

UKI="${1:?usage: assert-uki.sh <uki>}"

test -f "$UKI"

# PE files start with the DOS stub's "MZ".
head -c 2 "$UKI" | grep -q MZ || {
  echo "assert-uki: $UKI does not start with MZ; not a PE binary" >&2
  exit 1
}

for section in .linux .initrd .osrel .cmdline .uname .sbat; do
  if ! grep -qa -- "$section" "$UKI"; then
    echo "assert-uki: section $section not found in $UKI" >&2
    exit 1
  fi
done

echo "assert-uki: $UKI is a PE carrying all six UKI sections"
