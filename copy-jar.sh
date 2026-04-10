#!/bin/bash
# Builds the Rust WASM operating systems, then builds the mod JAR, and copies it to the project root

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# Build Rust WASM kernel (builds to workspace target at rust/target/)
echo "Building Rust OS..."
# Touch source to force recompilation (cargo sometimes misses changes in workspace builds)
touch rust/operating-system/rust/src/lib.rs
(cd rust && cargo build --release --target wasm32-unknown-unknown -p terminal-os)

# Build WASI programs (some may fail due to missing host functions — non-fatal)
echo "Building WASI programs..."
(cd rust && cargo build --release --target wasm32-wasip1 2>/dev/null || true)

# Copy WASM binaries to wasm-bin/
mkdir -p wasm-bin

# Kernel OS — always in workspace target dir
cp rust/target/wasm32-unknown-unknown/release/terminal_os.wasm wasm-bin/

# WASI programs — copy all .wasm from the workspace target dir
for f in rust/target/wasm32-wasip1/release/*.wasm; do
    [ -f "$f" ] && cp "$f" wasm-bin/
done

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
