#!/bin/sh
# Build this distribution's pm plugins and encode them as components.
#
# Two steps, because `cargo build --target wasm32-unknown-unknown` stops at a
# core module: wit-bindgen's canonical-ABI shims are in it, but nothing has
# wrapped it in a component yet. pm's own plugins/encoder does that, so this
# needs no `cargo install` and no wasm-tools -- only a Rust toolchain with the
# wasm32 target:
#
#   rustup target add wasm32-unknown-unknown
#
#   ./build.sh                 everything, into dist/
#   ./build.sh losos-image     just one
#
# The components then have to be signed before pm will load them. A plugin is
# code that runs inside pm and helps decide what a jail allows, so pm holds it
# to the same trust store as a build file:
#
#   pm sign  <config>/pm/plugins/losos-image.wasm
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$here"

target=wasm32-unknown-unknown
out="$here/dist"

# pm's encoder, from the sibling checkout this repository already assumes for
# the pm binary itself.
pm_root=${PM_ROOT:-$here/../../pm}
encoder="$pm_root/plugins/target/release/encoder"

crates=${*:-"losos-image losos-systemd"}

mkdir -p "$out"
cargo build --release --target "$target" $(for c in $crates; do echo "-p $c"; done)

if [ ! -x "$encoder" ]; then
    echo "building pm's component encoder in $pm_root/plugins" >&2
    ( cd "$pm_root/plugins" && cargo build --release -p encoder )
fi

for crate in $crates; do
    module="$here/target/$target/release/$(echo "$crate" | tr - _).wasm"
    "$encoder" "$module" "$out/$crate.wasm"
done

# Record what each component was built from, so tools/gates/plugins.py can tell
# a stale component from a current one. Not mtimes: git does not record them, so
# in a fresh clone every file is stamped at checkout time and a comparison
# decides by accident. The gate owns the hash function and writes it here, so
# there is one implementation.
python3 "$here/../tools/gates/plugins.py" --write-hashes

echo "built into $out"
