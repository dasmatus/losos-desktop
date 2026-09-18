#!/bin/sh
# Re-point the pkg-config metadata of a package that was just installed into the
# sysroot, so the next package in the same layer bundle can find it.
#
# A layer bundle builds many packages in one workspace (C5, C6). Each installs
# twice: into /dest, which is the shipped tree and must stay verbatim with
# prefix=/usr, and into /build/sysroot, which is what its siblings link against.
# The sysroot copy claims prefix=/usr too, because that is what it was
# configured with -- and a consumer reading it would be handed -I/usr/include,
# silently find the HOST's headers in pm's read-only /usr mirror, and compile
# against the wrong version with no warning at all.
#
# So the same rewrite sysroot.sh applies to unpacked dependencies is applied
# here to freshly installed ones. Same reasoning, different moment.
#
# Usage: sh stage-sysroot.sh <sysroot-dir>

set -eu

ROOT="${1:?usage: stage-sysroot.sh <sysroot-dir>}"

for dir in "$ROOT/usr/lib/pkgconfig" "$ROOT/usr/share/pkgconfig" \
           "$ROOT/usr/lib64/pkgconfig"; do
  [ -d "$dir" ] || continue
  find "$dir" -name '*.pc' -exec \
    sed -i "s|^prefix=/usr\$|prefix=$ROOT/usr|" {} +
done

# libtool archives and cmake package files carry absolute paths too, and a
# stale /usr in either is just as invisible.
find "$ROOT/usr" -name '*.la' -exec \
  sed -i "s|libdir='/usr/lib'|libdir='$ROOT/usr/lib'|g" {} + 2>/dev/null || true

echo "stage-sysroot: re-pointed metadata under $ROOT"
