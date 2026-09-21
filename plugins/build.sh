#!/bin/sh
# Compatibility wrapper for plugins/Justfile's build recipe.
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$(CDPATH= cd -- "$here/.." && pwd -P)
exec just --justfile "$here/Justfile" --working-directory "$repo" build "$@"
