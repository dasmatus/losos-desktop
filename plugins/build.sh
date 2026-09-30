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

# The same set as the Justfile's build recipe: this tree's plugins, then the
# ones pm's own tree carries that this OS needs; see there for why.
crates=${*:-"losos-image losos-mkosi losos-nix losos-systemd sysext sysupdate systemd"}

mkdir -p "$out"
ours= pms=
for c in $crates; do
    if [ -f "$here/$c/Cargo.toml" ]; then
        ours="$ours -p $c"
    elif [ -f "$pm_root/plugins/$c/Cargo.toml" ]; then
        pms="$pms -p $c"
    else
        echo "no plugin crate named $c here or in $pm_root/plugins" >&2
        exit 1
    fi
done
[ -z "$ours" ] || ( cd "$here" && cargo build --release --target "$target" $ours )
[ -z "$pms" ] || ( cd "$pm_root/plugins" && cargo build --release --target "$target" $pms )

if [ ! -x "$encoder" ]; then
    echo "building pm's component encoder in $pm_root/plugins" >&2
    ( cd "$pm_root/plugins" && cargo build --release -p encoder )
fi

for crate in $crates; do
    filename=$(printf '%s' "$crate" | tr '-' '_')
    if [ -f "$here/$crate/Cargo.toml" ]; then
        module="$here/target/$target/release/$filename.wasm"
    else
        module="$pm_root/plugins/target/$target/release/$filename.wasm"
    fi
    "$encoder" "$module" "$out/$crate.wasm"
done

echo "built into $out"
