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

# Build WASI programs. A failure in one program is non-fatal — some may
# intentionally fail due to missing host functions — but stderr is kept
# visible so real build breakage surfaces rather than being silently
# masked. The per-program copy loop below reports which outputs are
# missing.
echo "Building WASI programs..."
WASI_PKGS=()
for prog_dir in rust/wasm-programs/*/; do
    prog=$(basename "$prog_dir")
    WASI_PKGS+=("-p" "$prog")
done
(cd rust && cargo build --release --target wasm32-wasip1 "${WASI_PKGS[@]}" || true)

# Copy WASM binaries to wasm-bin/
mkdir -p wasm-bin
# Start from a clean set so manifest reflects this run's build outputs.
rm -f wasm-bin/*.wasm

# Kernel OS — always in workspace target dir
cp rust/target/wasm32-unknown-unknown/release/terminal_os.wasm wasm-bin/

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

# Build both MC-version jars via Stonecutter's chiseledBuild orchestrator.
# Each version subproject needs its own JAVA_HOME (Java 21 for MC 1.21.1,
# Java 25 for MC 26.1). Gradle's own JVM runs the daemon + plugins, so we
# point JAVA_HOME at Java 21 (works for both because moddev forks a
# toolchain-selected JVM for the actual compile of each subproject).
export JAVA_HOME="${JAVA_HOME:-/usr/lib/jvm/java-21-openjdk-amd64}"
./gradlew chiseledBuild

COPIED=0
for JAR in versions/*/build/libs/*.jar; do
    [ -f "$JAR" ] || continue
    case "$(basename "$JAR")" in
        *-sources.jar|*-javadoc.jar) continue ;;
    esac
    cp "$JAR" .
    echo "Copied $(basename "$JAR") to project root"
    COPIED=$((COPIED + 1))
done

if [ "$COPIED" -eq 0 ]; then
    echo "Error: No jars found in versions/*/build/libs/"
    exit 1
fi
