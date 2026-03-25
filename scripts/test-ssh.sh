#!/bin/bash
# SSH test suite for the Terminal OS simulator.
# KERN-041: Tests SSH key management, sshd server, and SSH client-server flow.
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

echo "=== Terminal OS SSH Tests ==="
echo ""

echo "--- SSH Key Management ---"

run_test "ssh-keygen generates key" "Key saved\|fingerprint" \
    "printf 'ssh-keygen\n' | timeout 20 cargo run --release -- --headless"

rm -rf simulator-data

run_test "passwd sets password" "Password updated\|updated" \
    "printf 'passwd\ntest123\ntest123\n' | timeout 15 cargo run --release -- --headless"

echo ""
echo "--- SSH Server ---"

rm -rf simulator-data

# sshd should start and print listening message.
# It will block waiting for connections, so timeout terminates it.
run_test "sshd starts listening" "listening on port" \
    "printf 'sshd 2222\n' | timeout 15 cargo run --release -- --headless"

echo ""
echo "--- SSH Client-Server (Two Instances) ---"

# Full two-instance SSH tests require:
#   - Instance 1 running sshd 22
#   - Instance 0 running ssh root@10.0.0.2
# The ssh client blocks waiting for interactive password input, which
# cannot be driven through the current headless stdin pipe.
# These tests are skipped until non-interactive auth (key-based) is supported.

skip_test "SSH connect between instances" "requires interactive password input"
skip_test "Remote command execution" "requires interactive password input"
skip_test "SSH disconnect" "requires interactive password input"
skip_test "Host key persistence" "requires multi-boot test"
skip_test "Multiple sequential connections" "requires interactive password input"

echo ""
echo "=== Results ==="
echo "  PASS: $PASS"
echo "  FAIL: $FAIL"
echo "  SKIP: $SKIP"
TOTAL=$((PASS + FAIL))
echo "  TOTAL: $TOTAL"

# Cleanup
rm -rf simulator-data

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
echo ""
echo "All tests passed! (${SKIP} skipped)"
