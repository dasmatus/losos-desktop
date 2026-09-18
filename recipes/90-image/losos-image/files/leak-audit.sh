#!/bin/sh
# Fail if the shipped tree still names a build path.
#
# The sysroot pattern rewrites absolute prefixes so a package can link against
# its siblings (share/sysroot.sh). If any of that leaks into the product, the
# failure is invisible until boot: a .pc file nobody reads again, or worse an
# RPATH pointing at /build/sysroot, which does not exist on the target and
# makes the binary fail to load with a message about a missing library rather
# than a missing directory.
#
# A design whose failure mode only appears at boot needs a gate that makes it
# appear at build time. This is that gate.
#
# Usage: sh leak-audit.sh <rootfs>

set -eu

ROOT="${1:?usage: leak-audit.sh <rootfs>}"

hits=0

# Metadata files are plain text and the cheapest place to catch it.
for pattern in '*.pc' '*.la' '*.cmake' '*-config'; do
  found=$(find "$ROOT" -name "$pattern" -exec grep -l '/build' {} + 2>/dev/null || true)
  if [ -n "$found" ]; then
    echo "leak-audit: build paths in metadata:" >&2
    echo "$found" | sed 's/^/  /' >&2
    hits=$((hits + 1))
  fi
done

# The one that actually breaks a boot: a build path baked into an ELF's
# dynamic section. grep over the binary is crude next to readelf -d, but
# readelf is not in pm's fingerprint table and this runs inside the jail.
elf_hits=$(find "$ROOT" -type f -perm -u+x -exec grep -l '/build/sysroot' {} + 2>/dev/null || true)
if [ -n "$elf_hits" ]; then
  echo "leak-audit: /build/sysroot embedded in executables:" >&2
  echo "$elf_hits" | sed 's/^/  /' >&2
  hits=$((hits + 1))
fi

if [ "$hits" -gt 0 ]; then
  echo "leak-audit: the shipped tree references the build sysroot" >&2
  exit 1
fi

echo "leak-audit: no build paths in the shipped tree"
