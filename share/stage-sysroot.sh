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
#
# libdir= alone is not enough. An archive that links another one names it in
# dependency_libs, and libtool writes that entry from the dependency's OWN
# libdir, not the staging prefix it was just installed under. tpm2-tss is where
# this surfaces: libtss2-esys links libtss2-sys, and `install` relinks the
# sysroot copy with -inst-prefix-dir so libdir= comes out correct -- while
# dependency_libs keeps the real prefix, `-ltss2-mu /usr/lib/libtss2-sys.la`.
# Every member of the bundle is in the same class, so this is fixed for all of
# them here rather than in one recipe.
#
# What reads that entry is a consumer that links the archive with libtool: it
# greps the file it names, and /usr is pm's read-only host mirror (C7), which
# has no libtss2-sys.la. gnutls finds libtss2-esys through pkg-config
# (`checking for tss2-esys... yes`), links it, and dies on
#
#     libtool: error: '/usr/lib/libtss2-sys.la' is not a valid libtool archive
#
# with the real file sitting at $ROOT/usr/lib/libtss2-sys.la the whole time.
#
# The anchor is a leading whitespace and the `lib` filename prefix, so the
# `-L/usr/lib` in the same variable is left alone -- that one is right, because
# the sysroot's .pc files were already re-pointed above and the linker path a
# consumer inherits comes from them.
find "$ROOT/usr" -name '*.la' -exec \
  sed -i -e "s|libdir='/usr/lib'|libdir='$ROOT/usr/lib'|g" \
         -e "s|\([[:space:]]\)/usr/lib/lib|\1$ROOT/usr/lib/lib|g" {} + 2>/dev/null || true

echo "stage-sysroot: re-pointed metadata under $ROOT"
