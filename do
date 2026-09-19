#!/usr/bin/env bash
# Compatibility wrapper around the Justfile entrypoint.
set -euo pipefail

repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
if ! command -v just >/dev/null 2>&1; then
  echo "no just binary on PATH" >&2
  echo "install it first: see $repo/docs/host-requirements.md" >&2
  exit 1
fi

exec just --justfile "$repo/Justfile" --working-directory "$repo" "$@"
