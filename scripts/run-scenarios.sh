#!/bin/bash
# Build the kernel + WASI programs, then run simulator scenarios.
#
#   scripts/run-scenarios.sh                 # every rust/simulator/scenarios/*.toml
#   scripts/run-scenarios.sh stp lacp        # scenarios whose file name contains a word
#   SKIP_BUILD=1 scripts/run-scenarios.sh    # don't rebuild the wasm first
#
# Scenarios run headless on the virtual clock; see rust/simulator/README.md.
set -e

export PATH="$HOME/.cargo/bin:$PATH"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RUST_DIR="$(cd "$SCRIPT_DIR/../rust" && pwd)"
cd "$RUST_DIR"

if [ -z "$SKIP_BUILD" ]; then
    echo "Building kernel (wasm32-unknown-unknown) ..."
    cargo build --release --target wasm32-unknown-unknown -p terminal-os
    echo "Building WASI programs (wasm32-wasip1) ..."
    PROGS=()
    for d in wasm-programs/*/; do
        PROGS+=("-p" "$(basename "$d")")
    done
    cargo build --release --target wasm32-wasip1 "${PROGS[@]}"
fi

cargo test --release -p terminal-simulator --test scenarios -- "$@"
