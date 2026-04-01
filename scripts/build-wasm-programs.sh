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

# Build all WASI programs at once via workspace
echo "Building all WASI programs..."
PROGS=()
for prog_dir in "$PROJECT_DIR"/wasm-programs/*/; do
    prog=$(basename "$prog_dir")
    PROGS+=("-p" "$prog")
done

if ! (cd "$PROJECT_DIR" && cargo build --target "$TARGET" --release "${PROGS[@]}" 2>&1); then
    echo "WASI program build failed!"
    exit 1
fi

# Copy all .wasm files from workspace target directory
PASS=0
FAIL=0
RELEASE_DIR="$PROJECT_DIR/target/$TARGET/release"

for prog_dir in "$PROJECT_DIR"/wasm-programs/*/; do
    prog=$(basename "$prog_dir")
    # The binary name might differ from the directory name (e.g., ssh-client -> ssh)
    # Check the Cargo.toml for the actual binary name
    BIN_NAME=$(grep -A1 '^\[\[bin\]\]' "$prog_dir/Cargo.toml" 2>/dev/null | grep 'name' | head -1 | sed 's/.*= *"\(.*\)".*/\1/')
    if [ -z "$BIN_NAME" ]; then
        BIN_NAME="$prog"
    fi

    WASM_FILE="$RELEASE_DIR/$BIN_NAME.wasm"
    if [ -f "$WASM_FILE" ]; then
        cp "$WASM_FILE" "$PROJECT_DIR/wasm-bin/"
        SIZE=$(stat -c%s "$PROJECT_DIR/wasm-bin/$BIN_NAME.wasm" 2>/dev/null || stat -f%z "$PROJECT_DIR/wasm-bin/$BIN_NAME.wasm" 2>/dev/null || echo "?")
        echo "  $BIN_NAME.wasm ($SIZE bytes) OK"
        PASS=$((PASS + 1))
    else
        echo "  $prog: FAIL (no .wasm output at $WASM_FILE)"
        FAIL=$((FAIL + 1))
    fi
done

# Also build the kernel OS
echo ""
echo "Building terminal_os.wasm..."
if (cd "$PROJECT_DIR" && cargo build --target wasm32-unknown-unknown --release -p terminal-os 2>&1 | tail -5); then
    cp "$PROJECT_DIR/target/wasm32-unknown-unknown/release/terminal_os.wasm" "$PROJECT_DIR/wasm-bin/"
    echo "  terminal_os.wasm OK"
    PASS=$((PASS + 1))
else
    echo "  terminal_os.wasm FAIL"
    FAIL=$((FAIL + 1))
fi

echo ""
echo "Built: $PASS, Failed: $FAIL"
if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
