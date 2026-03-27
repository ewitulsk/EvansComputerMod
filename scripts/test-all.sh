#!/bin/bash
# Master test suite — runs all test suites in order.
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TOTAL_PASS=0
TOTAL_FAIL=0

run_suite() {
    local name="$1"
    local script="$2"
    echo ""
    echo "=========================================="
    echo "  Running: $name"
    echo "=========================================="
    if bash "$SCRIPT_DIR/$script"; then
        echo "  Suite PASSED"
    else
        echo "  Suite FAILED"
        TOTAL_FAIL=$((TOTAL_FAIL + 1))
    fi
    TOTAL_PASS=$((TOTAL_PASS + 1))
}

echo "=== Terminal OS Master Test Suite ==="

run_suite "Process Tests" "test-processes.sh"
run_suite "SSH Tests" "test-ssh.sh"

# Networking tests require 2 instances, run if available
if [ -f "$SCRIPT_DIR/test-networking.sh" ]; then
    run_suite "Networking Tests" "test-networking.sh"
fi

echo ""
echo "=========================================="
echo "  All Suites Complete"
echo "  Suites run: $TOTAL_PASS"
echo "  Suite failures: $TOTAL_FAIL"
echo "=========================================="

if [ "$TOTAL_FAIL" -gt 0 ]; then
    exit 1
fi
echo "All suites passed!"
