#!/bin/bash
# Switch scenarios (replaces test-switch-phases-3-6.sh): CLI/show output,
# SVI management, switch.cfg replay, learning/flooding, VLANs, STP, LACP,
# Ctrl+T with a detached switch. Wraps scripts/run-scenarios.sh.
exec "$(dirname "$0")/run-scenarios.sh" switch vlan stp lacp ctrl_t "$@"
