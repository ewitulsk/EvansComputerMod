#!/bin/bash
# Networking scenarios: cables, faults, hubs, switch (learning, VLANs, STP,
# LACP), TCP/HTTP. Wraps scripts/run-scenarios.sh.
exec "$(dirname "$0")/run-scenarios.sh" two_hosts switch vlan stp lacp tcp faults "$@"
