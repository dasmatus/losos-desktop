#!/bin/sh
# Apply a package's patch series, in order, failing loudly on any that does not.
#
# A build step has no shell and no globbing (C1), so "apply every patch in this
# directory" cannot be written as pm `run:` commands -- the number of patches is
# not known when the recipe is written. This is the loop.
#
# Order is lexical, which is why the series is named 0001-, 0002-: a patch that
# depends on an earlier one is common, and "whatever readdir returned" is not an
# order.
#
# --forward and no fuzz on purpose. A patch that has already been applied, or
# that applies at an offset because upstream moved, is a patch that no longer
# means what its author meant. Both are failures here rather than warnings,
# because the alternative is a package that builds and quietly does not carry
# the change -- which for the factory-reset patch would mean a Settings panel
# with no reset button and a build that passed.
#
# Usage: sh apply-patches.sh <patch-dir> <source-dir>

set -eu

PATCHES="${1:?usage: apply-patches.sh <patch-dir> <source-dir>}"
SOURCE="${2:?usage: apply-patches.sh <patch-dir> <source-dir>}"

if [ ! -d "$PATCHES" ]; then
  echo "apply-patches: no patch directory at $PATCHES; nothing to do"
  exit 0
fi

applied=0
for patch in "$PATCHES"/*.patch; do
  # An unmatched glob stays literal in POSIX sh.
  [ -e "$patch" ] || continue
  echo "apply-patches: $(echo "$patch" | sed 's|.*/||')"
  patch --directory="$SOURCE" --strip=1 --forward --fuzz=0 --input="$patch"
  applied=$((applied + 1))
done

if [ "$applied" -eq 0 ]; then
  echo "apply-patches: $PATCHES exists but holds no *.patch"
  exit 0
fi

echo "apply-patches: applied $applied patch(es) to $SOURCE"
