#!/bin/sh
# Compile the proxy's decisions to WebAssembly, where api/proxy.js imports them.
#
# Vercel runs this as the project's build command, on a build image with no
# Rust, so it installs a pinned toolchain first when there is none -- the one
# network step, and the reason `cargo` is not assumed. Locally or in CI, the
# toolchain already on PATH is used.
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$here"

if ! command -v cargo >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --profile minimal --default-toolchain 1.95.0 \
            --target wasm32-unknown-unknown
    . "$HOME/.cargo/env"
fi

cargo build --release --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/losos_proxy.wasm api/losos_proxy.wasm

# Vercel wants an output directory even when every path is a function.
mkdir -p public
