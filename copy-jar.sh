#!/bin/bash
# Builds the Rust WASM operating systems, then builds the mod JAR, and copies it to the project root

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# Build Rust WASM projects
echo "Building Rust OS..."
(cd operating-system/rust && cargo build --release)
(cd operating-system/simple && cargo build --release --target wasm32-unknown-unknown)

# Copy WASM binaries to wasm-bin/
# Note: workspace builds output to root target/, not per-crate target/
mkdir -p wasm-bin
cp target/wasm32-unknown-unknown/release/terminal_os.wasm wasm-bin/
cp target/wasm32-unknown-unknown/release/simple.wasm wasm-bin/terminal.wasm

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
