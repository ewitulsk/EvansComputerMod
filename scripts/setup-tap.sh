#!/bin/bash
# Create a TAP device and set up NAT for simulator internet access.
# Must be run as root (or with CAP_NET_ADMIN).
#
# Usage: sudo scripts/setup-tap.sh [tap_name] [outgoing_interface]

set -e

TAP_NAME="${1:-tap0}"
TAP_IP="10.0.0.254"
TAP_SUBNET="10.0.0.0/24"
OUTGOING_IF="${2:-}"

# Auto-detect outgoing interface if not specified
if [ -z "$OUTGOING_IF" ]; then
    OUTGOING_IF=$(ip route | grep default | awk '{print $5}' | head -1)
    if [ -z "$OUTGOING_IF" ]; then
        echo "Error: Could not detect default network interface."
        echo "Usage: $0 [tap_name] [outgoing_interface]"
        exit 1
    fi
fi

echo "Setting up TAP device..."
echo "  TAP:      $TAP_NAME"
echo "  TAP IP:   $TAP_IP/24"
echo "  Outgoing: $OUTGOING_IF"

# Create TAP device
ip tuntap add dev "$TAP_NAME" mode tap
ip addr add "$TAP_IP/24" dev "$TAP_NAME"
ip link set dev "$TAP_NAME" up

# Enable IP forwarding
echo 1 > /proc/sys/net/ipv4/ip_forward

# NAT: masquerade traffic from the TAP subnet
iptables -t nat -A POSTROUTING -s "$TAP_SUBNET" -o "$OUTGOING_IF" -j MASQUERADE
iptables -A FORWARD -i "$TAP_NAME" -o "$OUTGOING_IF" -j ACCEPT
iptables -A FORWARD -i "$OUTGOING_IF" -o "$TAP_NAME" -m state --state RELATED,ESTABLISHED -j ACCEPT

echo ""
echo "TAP device $TAP_NAME configured successfully."
echo ""
echo "WASM instances should configure:"
echo "  IP:      10.0.0.X/24  (X = 1, 2, ...)"
echo "  Gateway: $TAP_IP"
echo "  DNS:     8.8.8.8"
echo ""
echo "Run simulator with: cargo run --release -- --tap $TAP_NAME --auto-net"
echo "Teardown with:      sudo scripts/teardown-tap.sh $TAP_NAME $OUTGOING_IF"
