#!/bin/bash
# Comprehensive networking test suite for the Terminal OS simulator.
# Tests: ping, HTTP server, curl, Python net module, TAP bridge.
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

# Clean storage
rm -rf simulator-data

echo "=== Terminal OS Networking Tests ==="
echo ""

# Create server script
echo "httpd 8080" > /tmp/test-server.sh

# Create server script with file
cat > /tmp/test-server-with-file.sh << 'SCRIPT'
python
import terminal
terminal.write_file("hello.txt", "Hello World from TerminalOS!")
exit()
httpd 8080
SCRIPT

echo "--- Ping Tests ---"

# Instance 1 just needs to be up with networking configured (auto-net handles it)
echo "" > /tmp/test-noop.sh
run_test "Ping between instances" "Reply from 10.0.0.2" \
    "printf 'ping 10.0.0.2 1\n' | timeout 20 cargo run --release -- --headless --instances 2 --auto-net --script '1:/tmp/test-noop.sh'"

echo ""
echo "--- HTTP Tests ---"

run_test "GET directory listing" "Terminal OS File Server" \
    "printf 'curl http://10.0.0.2:8080/\n' | timeout 20 cargo run --release -- --headless --instances 2 --auto-net --script '1:/tmp/test-server.sh'"

rm -rf simulator-data

run_test "GET specific file" "Hello World from TerminalOS" \
    "printf 'curl http://10.0.0.2:8080/hello.txt\n' | timeout 25 cargo run --release -- --headless --instances 2 --auto-net --script '1:/tmp/test-server-with-file.sh'"

rm -rf simulator-data

run_test "POST then GET file" "uploaded via POST" \
    "printf 'curl -X POST -d \"uploaded via POST\" http://10.0.0.2:8080/up.txt\ncurl http://10.0.0.2:8080/up.txt\n' | timeout 25 cargo run --release -- --headless --instances 2 --auto-net --script '1:/tmp/test-server.sh'"

rm -rf simulator-data

run_test "Verbose curl headers" "Server: TerminalOS" \
    "printf 'curl -v http://10.0.0.2:8080/\n' | timeout 20 cargo run --release -- --headless --instances 2 --auto-net --script '1:/tmp/test-server.sh'"

rm -rf simulator-data

echo ""
echo "--- Python Net Module Tests ---"

run_test "Python ping" "RTT:" \
    "printf 'python\nimport net\nrtt = net.ping(\"10.0.0.2\")\nprint(f\"RTT: {rtt}ms\")\nexit()\n' | timeout 20 cargo run --release -- --headless --instances 2 --auto-net"

rm -rf simulator-data

run_test "Python http_get" "Status: 200" \
    "printf 'python\nimport net\nresp = net.http_get(\"http://10.0.0.2:8080/\")\nprint(\"Status:\", resp[\"status\"])\nexit()\n' | timeout 25 cargo run --release -- --headless --instances 2 --auto-net --script '1:/tmp/test-server.sh'"

rm -rf simulator-data

run_test "Python http_post + http_get" "hello from python" \
    "printf 'python\nimport net\nnet.http_post(\"http://10.0.0.2:8080/t.txt\", b\"hello from python\")\nr = net.http_get(\"http://10.0.0.2:8080/t.txt\")\nprint(r[\"body\"])\nexit()\n' | timeout 30 cargo run --release -- --headless --instances 2 --auto-net --script '1:/tmp/test-server.sh'"

rm -rf simulator-data

echo ""
echo "--- TAP Bridge Tests (requires root) ---"

# Check if we can set up TAP (need root)
if [ "$(id -u)" = "0" ] || sudo -n true 2>/dev/null; then
    SUDO=""
    if [ "$(id -u)" != "0" ]; then
        SUDO="sudo"
    fi

    # Teardown any stale TAP first, then setup
    $SUDO bash "$SCRIPT_DIR/teardown-tap.sh" tap0 >/dev/null 2>&1 || true
    $SUDO bash "$SCRIPT_DIR/setup-tap.sh" tap0 >/dev/null 2>&1 || {
        echo "  (Failed to set up TAP device. Skipping TAP tests.)"
        echo ""
        echo "=== Results: $PASS passed, $FAIL failed ==="
        rm -f /tmp/test-server.sh /tmp/test-server-with-file.sh /tmp/test-noop.sh
        rm -rf simulator-data
        exit $([[ $FAIL -gt 0 ]] && echo 1 || echo 0)
    }

    run_test "Ping TAP gateway (10.0.0.254)" "Reply from 10.0.0.254" \
        "printf 'ping 10.0.0.254 1\n' | timeout 15 $SUDO cargo run --release -- --headless --auto-net --tap tap0"

    rm -rf simulator-data

    run_test "Ping real internet (8.8.8.8)" "Reply from 8.8.8.8" \
        "printf 'ping 8.8.8.8 1\n' | timeout 15 $SUDO cargo run --release -- --headless --auto-net --tap tap0"

    rm -rf simulator-data

    run_test "DNS resolve (example.com)" "Address:" \
        "printf 'nslookup example.com\n' | timeout 20 $SUDO cargo run --release -- --headless --auto-net --tap tap0"

    rm -rf simulator-data

    # Teardown TAP
    $SUDO bash "$SCRIPT_DIR/teardown-tap.sh" tap0 >/dev/null 2>&1
else
    echo "  (Skipped — requires root. Run with sudo to test TAP.)"
fi

echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="

# Cleanup
rm -f /tmp/test-server.sh /tmp/test-server-with-file.sh /tmp/test-noop.sh
rm -rf simulator-data

if [ $FAIL -gt 0 ]; then
    exit 1
fi
