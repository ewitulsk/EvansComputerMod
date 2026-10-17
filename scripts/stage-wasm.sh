#!/bin/bash
# Build the kernel and every WASI program, and stage them for the mod:
#   wasm-bin/*.wasm                         (bundled into the jar / dev runs)
#   wasm-bin/repo/                          (the Tech Village repository: Chat + the card example packages)
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

# lua's C sources (Lua 5.2), the browser's (QuickJS) and vim's (Vim 9.2) need
# wasi-sdk's clang and sysroot: WASI_SDK_PATH, or wasi-sdk unpacked in one of the
# usual places. Without it lua, browser and vim are skipped, loudly.
if [ -z "$WASI_SDK_PATH" ]; then
    for d in /opt/wasi-sdk "$HOME/wasi-sdk" /c/wasi-sdk "/c/Users/$USERNAME/wasi-sdk" /c/wasi-sdk-*; do
        if [ -x "$d/bin/clang" ] || [ -x "$d/bin/clang.exe" ]; then export WASI_SDK_PATH="$d"; break; fi
    done
fi
if [ -n "$WASI_SDK_PATH" ]; then
    export AR_wasm32_wasip1="$WASI_SDK_PATH/bin/llvm-ar"
else
    echo "warning: wasi-sdk not found (set WASI_SDK_PATH; https://github.com/WebAssembly/wasi-sdk/releases): lua, browser and vim will NOT be built"
    rm -f rust/target/wasm32-wasip1/release/lua.wasm rust/target/wasm32-wasip1/release/browser.wasm rust/target/wasm32-wasip1/release/vim.wasm
fi

echo "Building WASI programs..."
PKGS=()
for dir in rust/wasm-programs/*/; do
    prog="$(basename "$dir")"
    { [ "$prog" = lua ] || [ "$prog" = browser ] || [ "$prog" = vim ]; } && [ -z "$WASI_SDK_PATH" ] && continue
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

# Interactive GUI apps must compile before they run (ChicoryRuntime.SYNC_COMPILE_MAX_BYTES,
# 2 MiB): a bigger module starts interpreted and stays so while open, too slow to type into.
for app in browser; do
    f="wasm-bin/$app.wasm"
    if [ -f "$f" ] && [ "$(wc -c < "$f")" -gt 2097152 ]; then
        echo "error: $f is $(wc -c < "$f") bytes, over the 2 MiB compile-before-run limit (see the size profile in rust/Cargo.toml)"
        exit 1
    fi
done

# Vim's runtime files (syntax, filetype plugins, colours, help, defaults.vim) go with
# vim.wasm: the server's program folder is server-bin/ on every computer, so Vim
# finds them at /server-bin/vim/runtime (rust/wasm-programs/vim/port/pathdef.c).
rm -rf wasm-bin/vim
if [ -f wasm-bin/vim.wasm ]; then
    mkdir -p wasm-bin/vim
    cp -r rust/third_party/vim/runtime wasm-bin/vim/runtime
    cp rust/third_party/vim/LICENSE wasm-bin/vim/LICENSE
    cp rust/wasm-programs/vim/vimrc wasm-bin/vim/vimrc
fi

{
    ls wasm-bin/*.wasm | xargs -I{} basename {} | sort
    # Data files under wasm-bin/ (extracted next to the programs, WasmManager).
    if [ -d wasm-bin/vim ]; then (cd wasm-bin && find vim -type f | sort); fi
} > src/main/resources/wasm-bin/manifest.txt

# The Tech Village repository's packages (docs/TECH_VILLAGE_NETWORK.md "Apps"): Discord packed
# from the staged discord.wasm the way `discord --source` + `pkg build` do, natively (debug
# profile: a native release link of ecm-host-abi misses the host imports), plus the card
# example programs (examples/cards/*: bankd, pos, doord, skimmer, rx_nfc; docs/cards/CARDS.md),
# Python packages built from their folders the same way.
if [ -f wasm-bin/discord.wasm ]; then
    CARD_EXAMPLES=()
    for d in examples/cards/*/; do [ -f "$d/app.toml" ] && CARD_EXAMPLES+=("../${d%/}"); done
    (cd rust && cargo run --quiet -p discord --example village_repo -- ../wasm-bin/discord.wasm ../wasm-bin/repo "${CARD_EXAMPLES[@]}") ||
        { echo "error: the Tech Village repository package failed to build"; exit 1; }
fi
echo "Staged kernel + $ok programs into wasm-bin/ ($missing missing); manifest updated."
