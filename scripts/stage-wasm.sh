#!/bin/bash
# Build the kernel and every WASI program, and stage them for the mod:
#   wasm-bin/*.wasm                         (bundled into the jar / dev runs)
#   src/main/resources/wasm-bin/manifest.txt (lists what to extract at runtime)
#
# Used by copy-jar.sh, the GameTest runner (scripts/Test.ps1) and CI.
# A program that fails to build is reported but doesn't stop the others.
set -e

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
export PATH="$HOME/.cargo/bin:/c/Users/$USERNAME/.cargo/bin:$PATH"

echo "Building kernel (terminal_os.wasm)..."
(cd rust && cargo build --release --target wasm32-unknown-unknown -p terminal-os)

echo "Building WASI programs..."
PKGS=()
for dir in rust/wasm-programs/*/; do
    PKGS+=("-p" "$(basename "$dir")")
done
(cd rust && cargo build --release --target wasm32-wasip1 "${PKGS[@]}") || echo "warning: some WASI programs failed to build (see above)"

mkdir -p wasm-bin src/main/resources/wasm-bin
rm -f wasm-bin/*.wasm
cp rust/target/wasm32-unknown-unknown/release/terminal_os.wasm wasm-bin/

ok=0
missing=0
for dir in rust/wasm-programs/*/; do
    prog=$(basename "$dir")
    # The binary name can differ from the directory (ssh-client -> ssh).
    bin=$(grep -A1 '^\[\[bin\]\]' "$dir/Cargo.toml" 2>/dev/null | grep 'name' | head -1 | sed 's/.*= *"\(.*\)".*/\1/')
    [ -z "$bin" ] && bin="$prog"
    f="rust/target/wasm32-wasip1/release/$bin.wasm"
    if [ -f "$f" ]; then
        cp "$f" wasm-bin/
        ok=$((ok + 1))
    else
        echo "warning: no output for $prog ($f)"
        missing=$((missing + 1))
    fi
done

ls wasm-bin/*.wasm | xargs -I{} basename {} | sort > src/main/resources/wasm-bin/manifest.txt
echo "Staged kernel + $ok programs into wasm-bin/ ($missing missing); manifest updated."
