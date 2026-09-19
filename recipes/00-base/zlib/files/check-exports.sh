#!/bin/sh
# Fail if the staged libz.so exports nothing, or is missing a named symbol.
#
# `test -e libz.so` was the whole check here once, and it passed against a
# 13 KB shared library with an empty dynamic symbol table: -fvisibility=hidden
# had hidden zlib's entire public API and nothing in the recipe looked. What
# caught it was zlib's own test programs failing to link, which is coverage
# this recipe gets by accident and would lose the day anyone passes
# -DZLIB_BUILD_EXAMPLES=OFF to make the build faster.
#
# A step has no shell and no pipes (C1), which is why this is a script rather
# than `nm ... | grep`.
#
# Usage: sh check-exports.sh <nm> <library>

set -eu

NM="${1:?usage: check-exports.sh <nm> <library>}"
LIB="${2:?usage: check-exports.sh <nm> <library>}"

# Named rather than counted, and deflate specifically: it is the symbol the
# rest of the tree reaches zlib through, and a library exporting some other
# handful would satisfy a count.
for symbol in deflate inflate gzopen zlibVersion; do
  if ! "$NM" --dynamic --defined-only "$LIB" | grep -q "[ 	]$symbol\$\|[ 	]$symbol@"; then
    echo "check-exports: $LIB does not export $symbol"
    echo "check-exports: this is what -fvisibility=hidden does to a library"
    echo "check-exports: whose public API carries no visibility attribute."
    exit 1
  fi
done

echo "check-exports: $LIB exports its public API"
