#!/usr/bin/env bash
# Compatibility wrapper around the Justfile entrypoint.
set -euo pipefail

# `pwd -P`, not `pwd`: bash reports the logical path it was given, and on an
# ostree host that is `/home/<user>/...` where the real directory is
# `/var/home/<user>/...`. That path is handed to just as `--justfile`, just
# keeps an explicit `--justfile` verbatim, and `justfile_directory()` then
# carries the symlinked spelling into the container recipes. The Justfile
# canonicalises it again for exactly that reason; this keeps the wrapper from
# introducing the discrepancy in the first place, so its own error messages
# name the path everything else will use.
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
if ! command -v just >/dev/null 2>&1; then
  echo "no just binary on PATH" >&2
  echo "install it first: see $repo/docs/host-requirements.md" >&2
  exit 1
fi

if [ $# -eq 0 ]; then
  exec just --justfile "$repo/Justfile" --working-directory "$repo"
fi

case "$1" in
  -*)
    exec just --justfile "$repo/Justfile" --working-directory "$repo" "$@"
    ;;
esac

recipe=$1
shift
if [ $# -eq 0 ]; then
  exec just --justfile "$repo/Justfile" --working-directory "$repo" "$recipe"
fi

case "$1" in
  -*)
    exec just --justfile "$repo/Justfile" --working-directory "$repo" "$recipe" -- "$@"
    ;;
  *)
    exec just --justfile "$repo/Justfile" --working-directory "$repo" "$recipe" "$@"
    ;;
esac
