# Switch VLAN forwarding: access vs trunk, tagged vs untagged

What the in-game switch (`ecm-bridge`) does with a frame, by port mode and tagging. This describes the code as it is. The logic lives in `rust/crates/ecm-bridge/src/bridge.rs`:

- `ingress_vlan`: which VLAN a received frame belongs to, or whether it's dropped.
- `membership` and `egress`: which ports a VLAN leaves on, and whether it's tagged there.

For CLI syntax, see [switch_instructions.md](../switch_instructions.md).

## Terms

| Term | Meaning here |
|---|---|
| **Untagged** | No 802.1Q header. A **priority-tagged** frame (802.1Q with VID 0) counts as untagged; its PCP is kept. |
| **Tagged (VID n)** | Exactly one 802.1Q C-tag (TPID `0x8100`) with VID 1–4094. |
| **Access port** | `vlan access <id>`. It's a member of exactly one VLAN. A port becomes L2 in access VLAN 1 with `no routing`. |
| **Trunk port** | `vlan trunk native <id> [tag]` plus `vlan trunk allowed <list\|all>`. |
| **Native VLAN** | The VLAN that untagged frames on a trunk belong to. With `tag`, the native VLAN is tagged too, and untagged frames are refused. |
| **Allowed list** | The VLANs a trunk carries: `all` or an explicit list. |
| **Active VLAN** | The VLAN exists (`vlan <id>`) and isn't `shutdown`. VLAN 1 always exists and can't be shut down. |

## Ingress: which VLAN a received frame joins

| Port mode | Untagged / VID 0 | Tagged, VID = port's VLAN (access) / native (trunk) | Tagged, other VID *n* |
|---|---|---|---|
| **Access** VLAN *a* | VLAN *a* | **Dropped** (access ports accept no 802.1Q tags, even VID *a*) | **Dropped** |
| **Trunk**, native *v* untagged, allowed `all` | VLAN *v* | VLAN *v* | VLAN *n* |
| **Trunk**, native *v* untagged, allowed list *L* | VLAN *v* **only if *v* ∈ *L***, else dropped | VLAN *v* if *v* ∈ *L*, else dropped | VLAN *n* if *n* ∈ *L*, else dropped |
| **Trunk**, native *v* **`tag`** | **Dropped** | VLAN *v* if allowed | VLAN *n* if allowed |
| **Routed** (default, before `no routing`) | Not bridged; goes to the host IP stack | Not bridged | Not bridged |

After classification, the frame is dropped if its VLAN isn't **active**. That includes a VLAN accepted by `allowed all` that was never created. Every drop in this table increments the port's `rx_drops`.

## Egress: how a VLAN leaves each port

This applies to known-unicast forwarding and to flooding (broadcast, multicast, unknown unicast). Flooding goes to every forwarding member port of the VLAN except the one the frame came in on.

| Port mode | Frame's VLAN = port's VLAN (access) / native (trunk) | Frame's VLAN = other VLAN *n* |
|---|---|---|
| **Access** VLAN *a* | Sent **untagged** | Not sent |
| **Trunk**, native *v* untagged | Sent **untagged** (if *v* is allowed) | Sent **tagged *n*** if allowed. With `all`, only if VLAN *n* exists. |
| **Trunk**, native *v* **`tag`** | Sent **tagged *v*** (if *v* is allowed) | Sent **tagged *n*** if allowed |

- The tag is rewritten on egress: whatever tag the frame arrived with is stripped, and the egress port's rule applies. So a frame can arrive untagged on an access port and leave tagged on a trunk, or the other way round.
- The 802.1Q priority (PCP) from ingress is carried into the new tag. Untagged ingress gets PCP 0.
- A port in STP *Discarding* or *Learning*, or a LAG member that isn't *Distributing*, sends nothing. A *Learning* port still learns source MACs.

## Common combinations, end to end

Host A → switch port X → switch port Y → host B:

| A sends | Port X | Port Y | B receives |
|---|---|---|---|
| untagged | access 10 | access 10 | untagged ✔ |
| untagged | access 10 | access 20 | nothing (different VLAN) |
| untagged | access 10 | trunk native 1, allowed 10,20 | **tagged 10** |
| untagged | access 1 | trunk native 1, allowed 10,20 | nothing (**1 ∉ allowed**) |
| untagged | access 1 | trunk native 1, allowed `all` | untagged |
| tagged 10 | access 10 | anything | nothing (**dropped at X**) |
| tagged 10 | trunk native 1, allowed 10,20 | access 10 | untagged |
| tagged 30 | trunk native 1, allowed 10,20 | anything | nothing (dropped at X) |
| tagged 30 | trunk allowed `all`, VLAN 30 not created | anything | nothing (VLAN inactive) |
| untagged | trunk native 1 `tag` | anything | nothing (dropped at X) |

## Frames addressed to the switch itself (SVIs)

- A frame whose destination is the switch's own MAC, in a VLAN with an SVI (`interface vlan <id>` + `ip address`), goes to the switch's IP stack **untagged**. Broadcast and multicast in that VLAN go there too, as well as being flooded.
- With no SVI in that VLAN, such frames are dropped.
- Replies leave through the normal egress rules for that VLAN. That's how `switch_trunk` reaches sw2's VLAN 20 SVI across the tagged trunk.

## Hosts (`ifconfig <iface> vlan <id>|off`)

- A host NIC with `vlan N` tags everything it sends with N and accepts only frames tagged N.
- With `vlan off` (the default), it sends untagged and accepts untagged or VID-0 frames.
- So a tagged host has to be plugged into a **trunk** that allows N, not an access port. An access port drops all of its frames at ingress.

## Not handled

- **QinQ / S-tags** (TPID `0x88a8`) aren't recognised. A frame whose outer header is `0x88a8` is classified as untagged, and its payload (including any inner tags) is passed through unchanged.
- Only one C-tag is parsed. An inner second `0x8100` tag is treated as payload.
- **Control frames aren't VLAN-forwarded.** STP BPDUs, LACPDUs and LLDP are link-local. They're consumed by the port they arrive on, so they're unaffected by the tables above.

## Gotchas

- **`vlan trunk native <id>` resets the allowed list to `all`.** Set the native VLAN first, then `vlan trunk allowed`.
- **With an explicit allowed list, list the native VLAN too** if you want untagged traffic on the trunk. For example `vlan trunk native 1` with `vlan trunk allowed 1,10,20`.
- **An access port refuses a tag even when it matches its own VLAN.** Turn host tagging off (`ifconfig eth0 vlan off`) for hosts on access ports.

## See it happen

The debug scenarios exercise these paths on real blocks. `/ecm scenario spawn <name>` builds one in front of you and runs it; `commands <name>` prints the steps to type yourself. They're also GameTests: `scripts\Test.ps1 -GameTests ecm_switch`.

- `switch_vlans`: access ports in VLANs 10 and 20. Same-VLAN pings work; cross-VLAN pings get nothing, although it's the same subnet.
- `switch_trunk`: access → trunk (tagged) → access across two switches, plus a VLAN 20 SVI reached over the trunk.
