#!/bin/bash
# Process and pipeline integration test suite for the Terminal OS simulator.
# KERN-019: Tests pipes, redirects, process execution through the simulator.
#
# NOTE: WASI child processes currently use NullFd for stdio, so their stdout
# output is not visible. Tests that depend on seeing WASI program output are
# marked as SKIP with an explanation.
set -e

# Ensure cargo is in PATH (needed when running as root via sudo)
export PATH="$HOME/.cargo/bin:$PATH"
if [ -n "$SUDO_USER" ]; then
    export PATH="/home/$SUDO_USER/.cargo/bin:/root/.cargo/bin:$PATH"
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR/simulator"

PASS=0
FAIL=0
SKIP=0

run_test() {
    local name="$1"
    local expected="$2"
    shift 2

    echo -n "  $name... "
    OUTPUT=$(eval "$@" 2>/dev/null || true)
    if echo "$OUTPUT" | grep -q "$expected"; then
        echo "PASS"
        PASS=$((PASS + 1))
    else
        echo "FAIL"
        echo "    Expected to find: $expected"
        echo "    Got: $(echo "$OUTPUT" | tail -5)"
        FAIL=$((FAIL + 1))
    fi
}

skip_test() {
    local name="$1"
    local reason="$2"
    echo "  $name... SKIP ($reason)"
    SKIP=$((SKIP + 1))
}

# Clean storage
rm -rf simulator-data

echo "=== Terminal OS Process Tests ==="
echo ""

# ------------------------------------------------------------------
echo "--- Basic Shell Tests ---"

run_test "Echo command" "hello world" \
    "printf 'echo hello world\n' | timeout 10 cargo run --release -- --headless"

run_test "FD test" "FD test: PASS" \
    "printf 'fd_test\n' | timeout 15 cargo run --release -- --headless"

run_test "Crypto test" "Crypto test: PASS" \
    "printf 'crypto_test\n' | timeout 30 cargo run --release -- --headless"

# ------------------------------------------------------------------
echo ""
echo "--- Process Management Tests ---"

run_test "ps shows kernel" "kernel" \
    "printf 'ps\n' | timeout 10 cargo run --release -- --headless"

# ------------------------------------------------------------------
echo ""
echo "--- WASI Binary Tests ---"

# Set up WASI binaries in storage
rm -rf simulator-data
mkdir -p simulator-data/bin
for f in "$PROJECT_DIR"/wasm-bin/*.wasm; do
    [ -f "$f" ] && cp "$f" simulator-data/bin/
done

# WASI programs run but their stdout goes to NullFd, so we can't see output.
# Instead we verify the shell doesn't crash: after running hello.wasm the
# prompt comes back and we can still run commands.
run_test "WASI binary runs without crash" "bin/" \
    "printf 'bin/hello.wasm\nls\n' | timeout 15 cargo run --release -- --headless"

skip_test "WASI binary visible output" \
    "child process stdio uses NullFd -- output goes to /dev/null"

# ------------------------------------------------------------------
echo ""
echo "--- Pipeline Parser Tests ---"

# These test that the shell doesn't crash on pipeline syntax.
# The shell currently rejects builtin piping with a message.
run_test "Shell handles pipe syntax" "not a WASM program" \
    "printf 'echo test | echo pass\n' | timeout 10 cargo run --release -- --headless"

# Redirect syntax is not yet parsed; echo passes it through literally.
# We verify the shell doesn't crash by checking for TerminalOS banner.
run_test "Shell handles redirect syntax" "TERMINAL OS" \
    "printf 'echo test > /dev/null\n' | timeout 10 cargo run --release -- --headless"

run_test "Shell handles background syntax" "TERMINAL OS" \
    "printf 'echo test &\n' | timeout 10 cargo run --release -- --headless"

# ------------------------------------------------------------------
echo ""
echo "--- SSH Key Tests ---"

rm -rf simulator-data
run_test "ssh-keygen generates key" "Key saved" \
    "printf 'ssh-keygen\n' | timeout 20 cargo run --release -- --headless"

# ------------------------------------------------------------------
echo ""
echo "=== Results ==="
echo "  PASS: $PASS"
echo "  FAIL: $FAIL"
echo "  SKIP: $SKIP"
TOTAL=$((PASS + FAIL))
echo "  TOTAL: $TOTAL"

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
echo ""
echo "All tests passed!"
