# Tech Village validation

Validated on Windows on 2026-09-30 against staging `5144e55`, with the feature
changes present. Receipts are local, immutable files under `artifacts/`; the
runner retains failed runs as well as successful retests.

| Check | Executed result | Receipt directory |
|---|---|---|
| `ecm-net`, `ecm-router`, `ecm-bgp`, `terminal-os` | 119 Rust tests passed | `protocol-complete-20260930-002612` |
| Simulator scenarios 15, 16, 17 | All three passed | `protocol-complete-20260930-002612` |
| Minecraft 1.21.1 `ecm_router` | Six GameTests passed | `tech-village-release-20260930-002721` |
| Kernel host and Windows proxy | Six JUnit tests passed, none skipped | `host-storage-complete-20260930-002645` |
| Native Wasmtime interruption isolation | One JUnit test passed, none skipped | `native-cancellation-retest-20260930-001126` |
| Kernel ABI contract | 31 imports and 10 exports checked | `scripts/check-abi.py` |

The real-world GameTests check forwarding/NAT/DHCP and disabled-forwarding
controls; the same live instance detaching and reattaching; BGP rerouting and an
isolation control; generated fiber removal and crafted replacement with Sable
loaded; twenty infrastructure computers booting headless and fetching village
8's HTTP page from village 1; and a jigsaw village with fifteen distinct,
provisioned computer identities. Each case completes within its sub-minute
wall-clock limit. The HTTP and fiber tests found and verified fixes for storage
path normalization and placement during chunk promotion.

Pure tests cover forwarding errors, checksums, DHCP lease transitions, NAT
timeouts and quoted errors in both directions, BGP wire framing/capabilities,
AS sets, malformed UPDATE recovery, policy controls, best paths, and ring
withdrawals. Proxy integration tests use actual host TCP/UDP sockets and check
packet checksums, retransmission and rejection controls. Native isolation starts
two spinning Wasmtime instances and verifies that interrupting one leaves the
other running.

Reproduction commands and feature boundaries are in [the router guide](ROUTER.md)
and [TESTING.md](../TESTING.md). Both Minecraft jar variants and the Wasmtime
sidecar are packaged separately from the targeted test runs. Client rendering,
walking around the generated village, and the manual `/ecm techvillage tp 3`
interaction have not been tested in a graphical client. The automated tests
exercise placement and cross-village HTTP on the server.
