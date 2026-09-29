#!/bin/bash
# Smoke-tests for switch phases 3 (logging), 4 (LLDP), 5 (STP), 6 (LACP/LAG).
# Focused on CLI acceptance and show-command output, not full protocol convergence.
set -e

export PATH="$HOME/.cargo/bin:$PATH"
if [ -n "$SUDO_USER" ]; then
    export PATH="/home/$SUDO_USER/.cargo/bin:/root/.cargo/bin:$PATH"
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR/rust/simulator"

PASS=0
FAIL=0

run_test() {
    local name="$1"
    local expected="$2"
    local input="$3"
    local extra_args="${4:-}"

    echo -n "  $name... "
    rm -rf simulator-data
    OUTPUT=$(printf '%b' "$input" | timeout 20 cargo run --release -- --headless --instances 1 --auto-net $extra_args 2>&1 || true)
    if echo "$OUTPUT" | grep -q "$expected"; then
        echo "PASS"
        PASS=$((PASS + 1))
    else
        echo "FAIL"
        echo "    Expected: $expected"
        echo "    Got (tail):"
        echo "$OUTPUT" | tail -8 | sed 's/^/      /'
        FAIL=$((FAIL + 1))
    fi
}

echo "=== Switch Phases 3-6 Smoke Tests ==="

echo ""
echo "--- Phase 3: Logging ---"

run_test "show logging default state" "Logging severity: info" \
    "switch\nshow logging\nend\n"

run_test "logging severity change persists" "Logging severity: warning" \
    "switch\nlogging severity warning\nshow logging\nend\n"

run_test "vlan create emits log event" "VLAN 100 created" \
    "switch\nlogging severity debug\nvlan 100\nname TEST\nexit\nshow logging\nend\n"

run_test "clear logging wipes entries" "Entries: 0" \
    "switch\nvlan 200\nexit\nclear logging\nshow logging\nend\n"

run_test "show events alias" "severity" \
    "switch\nshow events\nend\n"

echo ""
echo "--- Phase 4: LLDP ---"

run_test "lldp enabled by default in show" "Enabled   : true" \
    "switch\nshow lldp configuration\nend\n"

run_test "lldp timer change" "Timer     : 10 s" \
    "switch\nlldp timer 10\nshow lldp configuration\nend\n"

run_test "lldp disable at top" "Enabled   : false" \
    "switch\nno lldp\nshow lldp configuration\nend\n"

run_test "lldp per-port receive off" "eth0    on   off" \
    "switch\ninterface eth0\nno lldp receive\nexit\nshow lldp configuration\nend\n"

run_test "show lldp neighbor-info empty" "no LLDP neighbors" \
    "switch\nshow lldp neighbor-info\nend\n"

run_test "show lldp statistics" "Rx-Err" \
    "switch\nshow lldp statistics\nend\n"

run_test "clear lldp neighbors" "switch(config)" \
    "switch\nclear lldp neighbors\nend\n"

echo ""
echo "--- Phase 5: Spanning Tree ---"

run_test "spanning-tree enabled show" "Spanning Tree: enabled" \
    "switch\nspanning-tree\nshow spanning-tree\nend\n"

run_test "bridge priority change" "Bridge ID  : 1000" \
    "switch\nspanning-tree\nspanning-tree priority 4096\nshow spanning-tree\nend\n"

run_test "forward-delay setting" "Hello/FD/MaxAge: 2/10/20" \
    "switch\nspanning-tree\nspanning-tree forward-delay 10\nshow spanning-tree\nend\n"

run_test "port bpdu-guard flag" "switch(config-if-eth0)" \
    "switch\ninterface eth0\nspanning-tree bpdu-guard\nexit\nend\n"

run_test "port cost change" "cost" \
    "switch\ninterface eth0\nspanning-tree cost 50000\nexit\nshow spanning-tree\nend\n"

run_test "no spanning-tree" "Spanning Tree: disabled" \
    "switch\nno spanning-tree\nshow spanning-tree\nend\n"

echo ""
echo "--- Phase 6: LACP / LAG ---"

run_test "show lacp configuration" "System Priority : 32768" \
    "switch\nshow lacp configuration\nend\n"

run_test "create lag enters context" "config-lag-1" \
    "switch\ninterface lag 1\nend\n"

run_test "lag with member port" "Members" \
    "switch\ninterface lag 1\nlacp mode active\nexit\ninterface eth0\nlag 1\nexit\nshow lacp aggregates\nend\n"

run_test "lag hash setting" "hash" \
    "switch\ninterface lag 1\nhash l3\nexit\nshow lacp aggregates\nend\n"

run_test "show interface lag" "LAG 1:" \
    "switch\ninterface lag 1\nexit\nshow interface lag 1\nend\n"

run_test "delete lag" "config-lag-1" \
    "switch\ninterface lag 1\nexit\nno interface lag 1\nend\n"

run_test "show lacp interfaces" "ActorState" \
    "switch\nshow lacp interfaces\nend\n"

echo ""
echo "=== Results: $PASS passed, $FAIL failed ==="

rm -rf simulator-data

if [ $FAIL -gt 0 ]; then
    exit 1
fi
