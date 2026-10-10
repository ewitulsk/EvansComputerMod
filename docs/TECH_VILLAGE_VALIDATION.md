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
walking around the generated village, and teleport were originally outside
that first server-only run. The follow-up below adds actual client checks.

## Playable labs, Blockbench assets and actual client validation

The follow-up against feature revision `8e713a2` with recorded uncommitted
changes was tested on Windows on 2026-09-30:

| Check | Executed result | Receipt directory |
|---|---|---|
| `ecm-net`, `ecm-router`, `terminal-os` | 112 Rust tests passed | `router-labs-complete-20260930-013625` (Rust step passed; the later GameTest step failed and was retested separately) |
| Playable `ecm_router_scenarios` | All ten GameTests passed, clean shutdown and no crash markers | `router-labs-clean-shutdown-20260930-014604` |
| WASI descriptor cancellation and kernel host | Seven JUnit tests passed, none skipped | `wasi-router-shutdown-20260930-014331` |
| Hidden real Minecraft client plus isolated normal-world server | Four paired render cases passed; six server markers including natural generation and actual scenario commands; clean exit | `tech-models-arrival-clean-exit-20260930-014832` |

The laboratories cover home DHCP/NAT and saved configuration restart, static
routes, completed two/three-hop traceroute, WAN DHCP/default route, static HTTP
port forwarding/hairpin and a closed port, two-AS BGP, ten-AS ring cut/isolation/
repair, prefix-policy denial and visible local preference/MED/community values,
real host TCP bridging, headless same-instance reattachment, six-way fiber
neighbor removal/repair and state serialization. Every lab is the same definition
available from `/ecm scenario spawn`; each completed inside its 55-second bound.

Four projects were authored and painted through the connected Blockbench MCP,
exported from checked-in `.bbmodel` sources, and inspected in native Blockbench
previews and Minecraft. The hidden client checks all new block models and the
module texture for missing assets. Removing a neighbor also removes exactly
six baked east-arm faces; repair restores them. Nonblank screenshots capture
both states and repair.

In fresh normal-world seed `73198425`, village 3 naturally generated fifteen
distinct provisioned computers. The actual `/ecm techvillage tp 3` command
arrived at `(4968, 68, -29)`, facing the ISP doorway. This run uses normal chunk
generation rather than `/place structure`. The server also executed the actual
scenario spawn, walkthrough, lead-list, lead down/up and clear commands.
Fixture cleanup settles for three seconds before shutdown so asynchronous
chunk IO completes before the server unload loop. Test worlds and all
screenshots remain under this repository; no desktop/profile/save automation
was used.

Failed receipts are retained. They exposed the WASI process-ID crash in
traceroute, hairpin forwarding rejecting its translated router source,
logical-proxy unicast delivery, commands being submitted during foreground
output, and socket cleanup cancellation escaping a child's finalizer. Those
paths now have executed positive and control checks. Already-generated worlds
without a valid village start are not backfilled by teleport: the command
reports the missing structure and cancels instead of placing the player along
fiber. Natural-generation/render checks target Minecraft 1.21.1; Minecraft 26.1
receives the shared router protocols, models and playable protocol labs.

The screenshots that used to follow this section showed the old plank village and
utility poles; both were replaced by the rebuild below.

## Vanilla villages, buried village cable and the generated fiber ring

The rebuild (branch `feature/tech-village-v2`) was tested on Windows on
2026-10-10. Every run uses natural generation in a fresh world, except the GameTest
world, which has structures disabled (there the village is generated at its planned
site the way `/place structure` does).

| Check | Executed result | Receipt directory |
|---|---|---|
| `FiberLineTest` (26.1 JUnit) | 5 tests: face-connected chords and drift, naive-diagonal control, packing, ring index, canonical arms, cut/repair bookkeeping | `fiber-line-20261010-034813` |
| Simulator `17_bgp_ring10` | Passed with the village port plan and 60/180 s timers: a cut reroutes within 10 s (carrier loss drops the session), isolation control | `ring-sim-20261010-023541` |
| `ecm_router` (1.21.1) | 6 GameTests, including the village network with a house PC online over the real cable, fiber cut/repair in a generated superflat chunk, and ten headless villages rerouting after cuts 8-9 and 2-3 | `tech-router-20261010-035445` |
| `ecm_router_scenarios` (1.21.1) | All 11 labs, including the pole-free `router_fiber` and the new `router_village` (real village cabling with village 5's files, cut-cable control) | `router-labs-20261010-040224` |
| Client suite, normal world seed 73198425 | Villages 2 (plains) and 3 (desert) natural; 4279-block chord walked; cut: traceroute 4 -> 12 hops; repair: back to 4; 11 paired screenshots | `tech-client-20261010-035540` |
| Client suite, superflat | Villages 1 and 2 (plains); 4305-block chord; same reroute; 7 paired screenshots | `tech-client-flat-20261010-035832` |
| Client suite, normal world seed 123456 | Plains and snowy villages; savanna ISP photographed; chord walk that first exposed the chunk-border arm bug | `tech-client-seed123456-20261010-035026` |

![Plains ISP grown into a vanilla village](images/tech-isp-plains.png)

![Desert ISP](images/tech-isp-desert.png)

![Taiga ISP](images/tech-isp-taiga.png)

![Savanna ISP](images/tech-isp-savanna.png)

![House router and PC with the patch cable over their UP faces](images/tech-house-desk.png)

![Fiber leaving the mast top in both directions](images/tech-fiber-mast.png)

![Fiber carving into a hillside](images/tech-fiber-terrain.png)

![Fiber floating over a valley](images/tech-fiber-valley.png)

![Fiber model fixture: free-standing riser on a patch panel and a six-way joint](images/tech-fiber-connected.png)

![Teleport arrival outside the ISP door, with the provisioned sign](images/tech-village-entrance.png)
