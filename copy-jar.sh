#!/bin/bash
# Builds the Rust WASM operating systems, then builds the mod JAR, and copies it to the project root

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# Sable physics integration is 1.21.1-only. The jar isn't on a Maven
# repo, so build.gradle reads it from libs/. Fetch it on demand here so
# fresh checkouts can build the 1.21.1 jar without a manual step.
SABLE_JAR="libs/sable-neoforge-1.21.1-1.1.3.jar"
SABLE_URL="https://cdn.modrinth.com/data/T9PomCSv/versions/g8CObHcP/sable-neoforge-1.21.1-1.1.3.jar"
fetch_sable() {
    if [ -f "$SABLE_JAR" ]; then
        return
    fi
    echo "Fetching Sable lib: $SABLE_JAR"
    mkdir -p libs
    if command -v curl >/dev/null 2>&1; then
        curl -fSL --retry 3 -o "$SABLE_JAR" "$SABLE_URL"
    elif command -v wget >/dev/null 2>&1; then
        wget -O "$SABLE_JAR" "$SABLE_URL"
    else
        echo "Error: need curl or wget to download $SABLE_URL" >&2
        exit 1
    fi
}

# The wasmtime sidecar subproject targets one MC version per build,
# selected by `-PmcVersion=...`. Each variant writes to its own build dir
# (build-mc1.21.1/, build-mc26.1/) so we can build both back-to-back.
# The 1.21.1 path of the main mod also requires the optional Sable jar —
# fetch_sable() pulls it in below before invoking Gradle.
#
# GRADLE_INVOCATIONS is a list of ;-delimited argument strings; each
# string becomes a separate `./gradlew` call so per-invocation -P
# properties don't bleed across builds.
GRADLE_INVOCATIONS=(
    "chiseledBuild"
    ":evanscomputermod-wasmtime:build -PmcVersion=26.1"
    ":evanscomputermod-wasmtime:build -PmcVersion=1.21.1"
)
JAR_GLOBS=(
    'versions/*/build/libs/*.jar'
    'evanscomputermod-wasmtime/build-mc*/libs/*.jar'
)
NEEDS_SABLE=1
for arg in "$@"; do
    case "$arg" in
        --1211)
            GRADLE_INVOCATIONS=(
                ":1.21.1:build"
                ":evanscomputermod-wasmtime:build -PmcVersion=1.21.1"
            )
            JAR_GLOBS=(
                'versions/1.21.1/build/libs/*.jar'
                'evanscomputermod-wasmtime/build-mc1.21.1/libs/*.jar'
            )
            NEEDS_SABLE=1
            ;;
        --261)
            GRADLE_INVOCATIONS=(
                ":26.1:build"
                ":evanscomputermod-wasmtime:build -PmcVersion=26.1"
            )
            JAR_GLOBS=(
                'versions/26.1/build/libs/*.jar'
                'evanscomputermod-wasmtime/build-mc26.1/libs/*.jar'
            )
            NEEDS_SABLE=0
            ;;
        *)
            echo "Unknown argument: $arg (expected --1211 or --261)"
            exit 1
            ;;
    esac
done

if [ "$NEEDS_SABLE" = "1" ]; then
    fetch_sable
fi

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

# Generate manifest listing all WASM files (for runtime extraction).
# Resources dir may be absent on a fresh checkout; ensure it exists.
mkdir -p src/main/resources/wasm-bin
ls wasm-bin/*.wasm 2>/dev/null | xargs -I{} basename {} | sort > src/main/resources/wasm-bin/manifest.txt
echo "WASM binaries copied to wasm-bin/, manifest updated"

# Build both MC-version jars via Stonecutter's chiseledBuild orchestrator.
# Each version subproject needs its own JAVA_HOME (Java 21 for MC 1.21.1,
# Java 25 for MC 26.1). Gradle's own JVM runs the daemon + plugins; the
# moddev toolchain provisions the per-version compiler JDK separately, so
# any reasonably-recent JDK works for Gradle's own JVM.
if [ -z "${JAVA_HOME:-}" ]; then
    if [ "$(uname)" = "Darwin" ] && command -v /usr/libexec/java_home >/dev/null 2>&1; then
        # macOS: pick whichever JDK the user has set up for command-line use.
        if HOME_DETECTED=$(/usr/libexec/java_home 2>/dev/null); then
            export JAVA_HOME="$HOME_DETECTED"
        fi
    elif [ -d /usr/lib/jvm/java-21-openjdk-amd64 ]; then
        # Common Debian/Ubuntu layout.
        export JAVA_HOME=/usr/lib/jvm/java-21-openjdk-amd64
    fi
fi
if [ -z "${JAVA_HOME:-}" ] || [ ! -d "$JAVA_HOME" ]; then
    echo "Error: could not detect a JDK for Gradle. Set JAVA_HOME to a JDK 17+ install." >&2
    exit 1
fi
echo "Using JAVA_HOME=$JAVA_HOME"
for INVOCATION in "${GRADLE_INVOCATIONS[@]}"; do
    # Word-split the invocation string into argv. Targets and -P args are
    # all simple tokens with no spaces inside them, so this is safe.
    # shellcheck disable=SC2086
    ./gradlew $INVOCATION
done

COPIED=0
for GLOB in "${JAR_GLOBS[@]}"; do
    for JAR in $GLOB; do
        [ -f "$JAR" ] || continue
        case "$(basename "$JAR")" in
            *-sources.jar|*-javadoc.jar) continue ;;
        esac
        cp "$JAR" .
        echo "Copied $(basename "$JAR") to project root"
        COPIED=$((COPIED + 1))
    done
done

if [ "$COPIED" -eq 0 ]; then
    echo "Error: No jars found under any of: ${JAR_GLOBS[*]}"
    exit 1
fi
