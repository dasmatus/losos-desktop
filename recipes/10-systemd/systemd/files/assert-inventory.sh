#!/bin/sh
# Assert that every component the option list claims to build is actually there.
#
# The systemd recipe turns on essentially everything upstream offers. That is
# easy to say and easy to stop being true: an option gets renamed, a dependency
# probe quietly fails and meson disables a feature rather than erroring, a
# component moves between split-bin paths. None of that fails the build -- it
# just produces a smaller systemd, and nobody notices until something does not
# start on a machine with no shell to debug it from.
#
# So the inventory is a test. Each line of the list is a path relative to the
# staged tree; a missing one names itself and fails the build.
#
# Usage: sh assert-inventory.sh <destdir> <inventory-file>

set -eu

DEST="${1:?usage: assert-inventory.sh <destdir> <inventory>}"
LIST="${2:?usage: assert-inventory.sh <destdir> <inventory>}"

missing=0
checked=0

while IFS= read -r line; do
  # Comments and blanks, so the inventory can be read by a person.
  case "$line" in
    ''|'#'*) continue ;;
  esac
  checked=$((checked + 1))

  # A line may list alternatives separated by '|'. Whether a given tool lands
  # in /usr/bin or /usr/lib/systemd depends on -Dsplit-bin and has moved
  # between releases, and this inventory is here to catch a component that is
  # ABSENT, not to pin down which directory upstream currently prefers. One
  # alternative present is a pass.
  found=0
  IFS='|'
  for candidate in $line; do
    if [ -e "$DEST/$candidate" ]; then
      found=1
      break
    fi
  done
  unset IFS

  if [ "$found" -eq 0 ]; then
    echo "inventory: MISSING $line" >&2
    missing=$((missing + 1))
  fi
done < "$LIST"

if [ "$missing" -gt 0 ]; then
  echo "inventory: $missing of $checked expected component(s) absent" >&2
  echo "inventory: a meson option was renamed, or a dependency probe failed" >&2
  echo "inventory: and meson disabled the feature instead of erroring." >&2
  exit 1
fi

echo "inventory: all $checked systemd components present"
