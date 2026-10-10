# Block and item models

These editable Java Block/Item projects were authored and painted through the
connected Blockbench MCP. Each `.bbmodel` embeds its texture so it can be opened
without this checkout's absolute texture paths. Preview PNGs (`<name>-preview.png`)
are native Blockbench renders; game screenshots are retained in test receipts.

## Tech infrastructure

| Project | Design |
|---|---|
| `fiber_span.bbmodel` | Sealed center coupler, identification band and six separate jacketed arms |
| `fiber_span_item.bbmodel` | The Fiber Span item: one straight jacketed strand with the gold identification band (like the copper wire item), on the block's texture |
| `fiber_patch_panel.bbmodel` | Full-block copper/fiber junction. Front: rack ears and screws, a row of aqua LC duplex adapters, a row of blue SC adapters, yellow LC jumpers dropping into a finger-duct cable manager, status LEDs. Back: copper side with RJ45 keystone jacks, blue patch boots, "CU" label and ground lug. A rubber cable gland on the centre of every other face, where a span's or cable's arm meets the panel |
| `network_cable_item.bbmodel` | The Network Cable item: a straight run through a node, the block's own center and arm geometry and texture |
| `interface_block_item.bbmodel` | The Interface item: the block's center with two arms, block texture |
| `always_on_module.bbmodel` | Green PCB, integrated circuit, traces, gold contacts and power indicator |
| `interface_probe.bbmodel` | 16x16 sprite for the Interface Probe (handheld tester with link screen and LEDs, blue lead, RJ45 plug); the project holds the sprite on a preview plane, the item stays `item/generated` |
| `screen_block.bbmodel` | Reference for the Screen's textures: the front ring and every other face use the Terminal's casing (`terminal_side`), shown next to a Terminal; the block model is `models/block/screen_block.json` (not exported from this project) |

Export the fiber span block, patch panel and always-on module with:

```powershell
py -3 scripts/gen-tech-assets.py
```

The exporter extracts the embedded PNGs, converts the project's UV coordinates
to Minecraft's 16-unit convention, and preserves cube geometry. Fiber cube names
start with `center`, `north`, `south`, `west`, `east`, `down` or `up`; the exporter
uses those names to produce multipart models. Preserve these prefixes when editing.
The patch panel gets four facing variants; its connection arms are not drawn (the
neighbouring span or cable draws its own arm into the gland).

The item-only projects (`fiber_span_item`, `network_cable_item`, `interface_block_item`)
are exported by `scripts/export-radio-models.py` (kind `item_ref`), which writes the
item model, including its display transforms, and leaves the shared block texture alone.

## Radio

The radio projects (SDRs, tuner, amplifiers, antennas, coax, dishes, ...) are exported
with `py -3 scripts/export-radio-models.py [name ...]`; its docstring lists the kinds.

| Project | Design |
|---|---|
| `sdr_basic/standard/advanced.bbmodel` | Full-block receiver. Back: N-type antenna jack in the centre (where a coax arm lands) inside a coloured ring (green = receive only on Basic, red = transmit-capable) under an antenna glyph. SMA sockets in the centre of both sides and the top, because coax connects on any side. Rubber-duck whip (the built-in antenna) on a rear top corner. Tiers keep their casing colours (charcoal, navy, purple); Basic has one RX LED, Standard RX/TX, Advanced RX/TX/REF, a wider display and a second knob |
| `antenna_tuner.bbmodel` | Full-block tuner: cross-needle meter, TRANSMITTER/ANTENNA knobs and band switch on the front; "TX" jack with red ring on the back; brass ANT jack on a ceramic feedthrough on top; SMA sockets on both sides |

Validate in Minecraft with `scripts/Test.ps1 -Area radio-render -ClientChecks
-ClientSuite radio -NoStage`: besides every radio block and item it builds close-ups
of the SDR jacks and tuner with coax plugged in, patch panels with fiber and copper
attached, and a Screen cluster next to a Terminal, and shows the network items in frames.
