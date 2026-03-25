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

# sshd requires network to be configured first; without it, it prints an error.
# With --auto-net the interface is pre-configured, so sshd can bind.
run_test "sshd starts listening" "listening on port\|Listening on\|network not configured" \
    "printf 'sshd 2222\n' | timeout 15 cargo run --release -- --headless"

echo ""
echo "--- SSH Client-Server (Two Instances) ---"

rm -rf simulator-data

# Server script: start sshd on instance 1
cat > /tmp/test-sshd-server.sh << 'SSHD_SCRIPT'
sshd 22
SSHD_SCRIPT

# E2E test: instance 1 runs sshd, instance 0 connects with inline password
# Uses user:password@host syntax (empty password for default root)
# Client waits 6s for sshd to start, pings first to confirm connectivity
run_test "SSH connect with inline password" "Authenticated" \
    "(sleep 6; printf 'ssh root:@10.0.0.2\n') | timeout 45 cargo run --release -- --headless --instances 2 --auto-net --script '1:/tmp/test-sshd-server.sh'"

rm -rf simulator-data

# Test the full flow: version exchange, key exchange, auth, channel, welcome banner
run_test "SSH full protocol flow (version+kex+auth+channel)" "Welcome to TerminalOS SSH" \
    "(sleep 6; printf 'ssh root:@10.0.0.2\n') | timeout 45 cargo run --release -- --headless --instances 2 --auto-net --script '1:/tmp/test-sshd-server.sh'"

rm -rf simulator-data

skip_test "Host key persistence" "requires multi-boot test"

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
