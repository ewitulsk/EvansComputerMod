#!/bin/bash
# Builds the Rust WASM operating systems, then builds the mod JAR, and copies it to the project root

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# Build Rust WASM kernel
echo "Building Rust OS..."
(cd rust/operating-system/rust && cargo build --release)

# Build WASI programs (these may have link errors for some programs — non-fatal)
echo "Building WASI programs..."
(cd rust && cargo build --release --target wasm32-wasip1 2>/dev/null || true)

# Copy WASM binaries to wasm-bin/
mkdir -p wasm-bin

# Kernel OS (built in per-crate target dir due to [build] target in .cargo/config.toml)
cp rust/operating-system/rust/target/wasm32-unknown-unknown/release/terminal_os.wasm wasm-bin/

# WASI programs — copy all .wasm from the workspace target dir
for f in rust/target/wasm32-wasip1/release/*.wasm; do
    [ -f "$f" ] && cp "$f" wasm-bin/
done

echo "WASM binaries copied to wasm-bin/"

# Build mod
./gradlew build

JAR=$(find build/libs -name "*.jar" ! -name "*-sources.jar" ! -name "*-javadoc.jar" | head -1)

if [ -z "$JAR" ]; then
    echo "Error: No jar found in build/libs/"
    exit 1
fi

cp "$JAR" .
echo "Copied $(basename "$JAR") to project root"
