#!/bin/sh
# Recreate musl's dynamic-loader entry in a staged usr/lib.
#
# sysroot.sh stages only usr/, but the musl loader the kernel execs is named by
# an absolute path in every ELF's PT_INTERP: /lib/ld-musl-<arch>.so.1. The tree
# is usr-merged below (/lib -> /usr/lib), so /usr/lib/ld-musl-<arch>.so.1 is the
# one place that keeps that absolute path valid. Nothing else creates it.
#
# Usage: sh loader-link.sh <libdir> <arch>
#
# pm has no shell (C1), so this logic cannot live in a run: line; it is a script
# invoked as two plain words.

set -eu

LIBDIR="${1:?usage: loader-link.sh <libdir> <arch>}"
ARCH="${2:?usage: loader-link.sh <libdir> <arch>}"

loader="$LIBDIR/ld-musl-$ARCH.so.1"

if [ ! -e "$loader" ] && [ ! -L "$loader" ]; then
  test -f "$LIBDIR/libc.so"
  ln -s libc.so "$loader"
fi
