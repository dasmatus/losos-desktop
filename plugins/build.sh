#!/bin/sh
# Compatibility wrapper for plugins/Justfile's build recipe.
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$(CDPATH= cd -- "$here/.." && pwd -P)
target=wasm32-unknown-unknown
out="$here/dist"
pm_root=${PM_ROOT:-$repo/../pm}
encoder="$pm_root/plugins/target/release/encoder"

if command -v just >/dev/null 2>&1; then
    exec just --justfile "$here/Justfile" --working-directory "$repo" build "$@"
fi

crates=${*:-"losos-image losos-mkosi losos-systemd"}

mkdir -p "$out"
cargo build --release --target "$target" $(for c in $crates; do echo "-p $c"; done)

if [ ! -x "$encoder" ]; then
    echo "building pm's component encoder in $pm_root/plugins" >&2
    ( cd "$pm_root/plugins" && cargo build --release -p encoder )
fi

for crate in $crates; do
    filename=$(printf '%s' "$crate" | tr '-' '_')
    module="$here/target/$target/release/$filename.wasm"
    "$encoder" "$module" "$out/$crate.wasm"
done

echo "built into $out"
