#!/bin/bash
# Remove a TAP device and associated iptables rules.
# Must be run as root.
#
# Usage: sudo scripts/teardown-tap.sh [tap_name] [outgoing_interface]

TAP_NAME="${1:-tap0}"
TAP_SUBNET="10.0.0.0/24"
OUTGOING_IF="${2:-}"

# Auto-detect outgoing interface if not specified
if [ -z "$OUTGOING_IF" ]; then
    OUTGOING_IF=$(ip route | grep default | awk '{print $5}' | head -1)
fi

echo "Tearing down TAP device $TAP_NAME..."

# Remove iptables rules (ignore errors if they don't exist)
iptables -t nat -D POSTROUTING -s "$TAP_SUBNET" -o "$OUTGOING_IF" -j MASQUERADE 2>/dev/null || true
iptables -D FORWARD -i "$TAP_NAME" -o "$OUTGOING_IF" -j ACCEPT 2>/dev/null || true
iptables -D FORWARD -i "$OUTGOING_IF" -o "$TAP_NAME" -m state --state RELATED,ESTABLISHED -j ACCEPT 2>/dev/null || true

# Remove TAP device
ip link delete "$TAP_NAME" 2>/dev/null || true

echo "Done."
