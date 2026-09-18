#!/bin/sh
# Assert the four modules exist under the kernel version that built them.
#
# An out-of-tree module installed under the wrong kernel version is invisible
# to modprobe and produces no error anywhere -- the GPU simply never
# initialises. Naming the version here ties the check to the kernel this layer
# actually built.
set -eu

DEST="${1:?usage: assert-modules.sh <destdir> <kver>}"
KVER="${2:?usage: assert-modules.sh <destdir> <kver>}"

missing=0
for module in nvidia nvidia-drm nvidia-modeset nvidia-uvm; do
  if ! find "$DEST/usr/lib/modules/$KVER" -name "$module.ko*" | grep -q .; then
    echo "nvidia-open: $module.ko not installed under $KVER" >&2
    missing=$((missing + 1))
  fi
done

[ "$missing" -eq 0 ] || exit 1
echo "nvidia-open: all four modules present under $KVER"
