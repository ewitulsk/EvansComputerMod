#!/bin/bash
# Builds the Rust WASM operating systems, then builds the mod JAR, and copies it to the project root

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# Build Rust WASM kernels (builds to workspace target at rust/target/)
echo "Building Rust OS kernels..."
# Touch sources to force recompilation (cargo sometimes misses changes in workspace builds)
touch rust/operating-system/rust/src/lib.rs
touch rust/switch-os/src/lib.rs
(cd rust && cargo build --release --target wasm32-unknown-unknown -p terminal-os -p switch-os)

# Build WASI programs (some may fail due to missing host functions — non-fatal)
echo "Building WASI programs..."
WASI_PKGS=()
for prog_dir in rust/wasm-programs/*/; do
    prog=$(basename "$prog_dir")
    WASI_PKGS+=("-p" "$prog")
done
(cd rust && cargo build --release --target wasm32-wasip1 "${WASI_PKGS[@]}" 2>/dev/null || true)

# Copy WASM binaries to wasm-bin/
mkdir -p wasm-bin
# Start from a clean set so manifest reflects this run's build outputs.
rm -f wasm-bin/*.wasm

# Kernel OSes — always in workspace target dir
cp rust/target/wasm32-unknown-unknown/release/terminal_os.wasm wasm-bin/
cp rust/target/wasm32-unknown-unknown/release/switch_os.wasm wasm-bin/

# WASI programs — copy each program listed in rust/wasm-programs/
PASS=0
FAIL=0
for prog_dir in rust/wasm-programs/*/; do
    prog=$(basename "$prog_dir")
    # Some programs use a different binary name than the directory (e.g., ssh-client -> ssh).
    BIN_NAME=$(grep -A1 '^\[\[bin\]\]' "$prog_dir/Cargo.toml" 2>/dev/null | grep 'name' | head -1 | sed 's/.*= *"\(.*\)".*/\1/')
    if [ -z "$BIN_NAME" ]; then
        BIN_NAME="$prog"
    fi

    WASM_FILE="rust/target/wasm32-wasip1/release/$BIN_NAME.wasm"
    if [ -f "$WASM_FILE" ]; then
        cp "$WASM_FILE" wasm-bin/
        PASS=$((PASS + 1))
    else
        echo "Warning: missing WASI output for $prog ($WASM_FILE)"
        FAIL=$((FAIL + 1))
    fi
done

echo "WASI copy summary: $PASS copied, $FAIL missing"

# Generate manifest listing all WASM files (for runtime extraction)
ls wasm-bin/*.wasm 2>/dev/null | xargs -I{} basename {} | sort > src/main/resources/wasm-bin/manifest.txt
echo "WASM binaries copied to wasm-bin/, manifest updated"

# Build mod
./gradlew build

JAR=$(find build/libs -name "*.jar" ! -name "*-sources.jar" ! -name "*-javadoc.jar" | head -1)

if [ -z "$JAR" ]; then
    echo "Error: No jar found in build/libs/"
    exit 1
fi

cp "$JAR" .
echo "Copied $(basename "$JAR") to project root"
