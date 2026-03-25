#!/bin/bash
# Build all WASI programs for Terminal OS.
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# Ensure wasm32-wasip1 target is available
if ! rustup target list --installed | grep -q wasm32-wasip1; then
    echo "Installing wasm32-wasip1 target..."
    rustup target add wasm32-wasip1
fi

TARGET="wasm32-wasip1"
mkdir -p "$PROJECT_DIR/wasm-bin"

PASS=0
FAIL=0

for prog_dir in "$PROJECT_DIR"/wasm-programs/*/; do
    prog=$(basename "$prog_dir")
    echo -n "Building $prog... "
    if (cd "$prog_dir" && cargo build --target "$TARGET" --release 2>/dev/null); then
        cp "$prog_dir/target/$TARGET/release/$prog.wasm" "$PROJECT_DIR/wasm-bin/"
        SIZE=$(stat -f%z "$PROJECT_DIR/wasm-bin/$prog.wasm" 2>/dev/null || stat -c%s "$PROJECT_DIR/wasm-bin/$prog.wasm" 2>/dev/null || echo "?")
        echo "OK ($SIZE bytes)"
        PASS=$((PASS + 1))
    else
        echo "FAIL"
        FAIL=$((FAIL + 1))
    fi
done

# Also build the kernel OS
echo -n "Building terminal_os.wasm... "
if (cd "$PROJECT_DIR/operating-system/rust" && cargo build --target wasm32-unknown-unknown --release 2>/dev/null); then
    cp "$PROJECT_DIR/operating-system/rust/target/wasm32-unknown-unknown/release/terminal_os.wasm" "$PROJECT_DIR/wasm-bin/"
    echo "OK"
    PASS=$((PASS + 1))
else
    echo "FAIL"
    FAIL=$((FAIL + 1))
fi

echo ""
echo "Built: $PASS, Failed: $FAIL"
if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
