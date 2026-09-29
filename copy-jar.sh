#!/bin/bash
# Builds the Rust WASM operating systems, then builds the mod JAR, and copies it to the project root

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# The 1.21.1 build compiles against Sable and Create, which aren't on a
# Maven repo. scripts/fetch-libs.sh downloads them into libs/ on demand
# so fresh checkouts can build the 1.21.1 jar without a manual step.
fetch_sable() {
    bash scripts/fetch-libs.sh
}

# The wasmtime sidecar subproject targets one MC version per build,
# selected by `-PmcVersion=...`. Each variant writes to its own build dir
# (build-mc1.21.1/, build-mc26.1/) so we can build both back-to-back.
# The 1.21.1 path of the main mod also requires the optional Sable and
# Create jars; fetch_sable() pulls them in below before invoking Gradle.
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

# Build the kernel + WASI programs and stage them into wasm-bin/ (+ manifest).
bash scripts/stage-wasm.sh

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
