#!/bin/bash
# Build the kernel and every WASI program, and stage them for the mod:
#   wasm-bin/*.wasm                         (bundled into the jar / dev runs)
#   wasm-bin/repo/                          (the Tech Village repository's Chat package)
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

# lua's C sources (Lua 5.2) need wasi-sdk's clang and sysroot: WASI_SDK_PATH, or
# wasi-sdk unpacked in one of the usual places. Without it lua is skipped, loudly.
if [ -z "$WASI_SDK_PATH" ]; then
    for d in /opt/wasi-sdk "$HOME/wasi-sdk" /c/wasi-sdk "/c/Users/$USERNAME/wasi-sdk"; do
        if [ -x "$d/bin/clang" ] || [ -x "$d/bin/clang.exe" ]; then export WASI_SDK_PATH="$d"; break; fi
    done
fi
if [ -n "$WASI_SDK_PATH" ]; then
    export AR_wasm32_wasip1="$WASI_SDK_PATH/bin/llvm-ar"
else
    echo "warning: wasi-sdk not found (set WASI_SDK_PATH; https://github.com/WebAssembly/wasi-sdk/releases): lua will NOT be built"
    rm -f rust/target/wasm32-wasip1/release/lua.wasm
fi

echo "Building WASI programs..."
PKGS=()
for dir in rust/wasm-programs/*/; do
    prog="$(basename "$dir")"
    [ "$prog" = lua ] && [ -z "$WASI_SDK_PATH" ] && continue
    PKGS+=("-p" "$prog")
done
# A failed build must stop here: staging would otherwise ship whatever stale
# .wasm the target directory still holds (python.wasm went out without its
# radio module that way).
(cd rust && cargo build --release --target wasm32-wasip1 "${PKGS[@]}") || { echo "error: WASI programs failed to build (see above); nothing staged"; exit 1; }

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

# The Tech Village repository's package (docs/TECH_VILLAGE_NETWORK.md "Apps"): Chat packed
# from the staged chatapp.wasm the way `chatapp --source` + `pkg build` do, natively (debug
# profile: a native release link of ecm-host-abi misses the host imports).
if [ -f wasm-bin/chatapp.wasm ]; then
    (cd rust && cargo run --quiet -p chatapp --example village_repo -- ../wasm-bin/chatapp.wasm ../wasm-bin/repo) ||
        { echo "error: the Tech Village repository package failed to build"; exit 1; }
fi
echo "Staged kernel + $ok programs into wasm-bin/ ($missing missing); manifest updated."
