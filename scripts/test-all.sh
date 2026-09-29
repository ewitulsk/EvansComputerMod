#!/bin/bash
# Everything the simulator can check: its unit tests plus every scenario.
# (Crate tests: cargo test -p ecm-net -p ecm-bridge -p terminal-os; Java host:
# ./gradlew :26.1:test.)
set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"
"$SCRIPT_DIR/run-scenarios.sh"
cd "$SCRIPT_DIR/../rust" && cargo test --release -p terminal-simulator --bins
