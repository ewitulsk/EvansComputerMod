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
| `sdr_basic/standard/advanced.bbmodel` | Full-block receiver. Back: N-type antenna jack in the centre (where a coax arm lands) inside a coloured ring (green = receive only on Basic, red = transmit-capable) under an antenna glyph. SMA sockets in the centre of both sides and the top. Coax connects only at these four jacks (not the front or bottom). Rubber-duck whip (the built-in antenna) on a rear top corner. Tiers keep their casing colours (charcoal, navy, purple); Basic has one RX LED, Standard RX/TX, Advanced RX/TX/REF, a wider display and a second knob |
| `antenna_tuner.bbmodel` | Full-block tuner: cross-needle meter, TRANSMITTER/ANTENNA knobs and band switch on the front; "TX" jack with red ring on the back (signal in); brass ANT jack on a ceramic feedthrough on top (signal out); louvred sides with no jacks |
| `amplifier_100w/1kw/10kw.bbmodel` | Two textures: the tier's panel (`amplifier_<tier>.png`, 128 px: front, back, side, top) and the shared `amplifier_parts.png` (gold N jacks, terminal block, posts, fins). Front: "RF OUT" N jack in the centre (y 6..10, reaching the block edge where a coax arm lands), bar-graph display in the tier colour (green/amber/red) and PWR/TX/ALARM LEDs. Back: "RF IN" N jack, "DC>" supply label. Right side (seen from behind): FE terminal block with red/black posts at the block edge. The 10 kW case is a full block; its jacks sit proud of the faces like the SDR's |
| `burner_generator.bbmodel` | Firebox door on the front, chimney on top, a terminal block with red/black posts on both sides and the back: the faces it pushes FE through (`burner_generator_terminals.png`) |
| `access_point.bbmodel` | White case (5 px) with two rear antennas; a raised RJ45 housing on the back whose port is centred at y 6..10 where a cable arm runs, and a second RJ45 port on the underside |
| `microwave_radio.bbmodel` | Outdoor unit: sealed RJ45 gland on the front (to the block edge), RJ45 port underneath, waveguide flange on the back that meets a dish's mounting post |
| `dish_medium.bbmodel` | The 1.2 m dish stands on two posts, one centred in each bottom block, joined by a brace, so a radio behind either bottom block meets a post (small and large dishes have one centred post) |
| `lightning_arrestor.bbmodel` | Grey body with a lightning glyph; gold N-connector collars step each arm down to the coax width. No ground lug: the block needs no ground connection |
| `feed_point.bbmodel` | Centre with lugs on its axis and coax arms on the other sides; the item shows lugs east-west and coax north-south |
| `sensor_wire_item.bbmodel` | The Sensor Wire item: a three-turn coil of wire with a lead to a black plug with a gold tip (`textures/item/sensor_wire_item.png`) |
| `lidar_sensor_item.bbmodel` | The Lidar Sensor item: the floor-mount base, connector, head and cap, on the block's texture |

The terminal's RJ45 keystone in an open bay's divider (where a cable plugs into a face
with an open bay) is generated with the bays by `scripts/gen-module-models.py` and
`scripts/gen-module-textures.py`.

Validate in Minecraft with `scripts/Test.ps1 -Area radio-render -ClientChecks
-ClientSuite radio -NoStage`: besides every radio block and item it builds close-ups
of the SDR jacks and tuner with coax plugged in, patch panels with fiber and copper
attached, and a Screen cluster next to a Terminal, amplifier chains with coax and a
generator on each terminal block (plus coax in front of an SDR as a no-arm control), Access
Points cabled on their back and bottom ports (and a cable beside one, no arm), a microwave
radio on a cable feeding a 1.2 m dish, a Terminal with a cable in an open bay, the
arrestor, and two Lidar Sensors joined by Sensor Wire, and shows the network items in frames.

## Cellular

Built through the Blockbench MCP (textures painted procedurally, 64x64 atlases at one texel per model
pixel, embedded in each project), exported with `py -3 scripts/export-radio-models.py <name>`.

| Project | Kind | Design |
|---|---|---|
| `lte_antenna.bbmodel` | facing | White sector-panel radome with grey end caps, aluminium back tray, mounting bracket and pole clamps on the mast side |
| `mmwave_antenna.bbmodel` | facing | Dark phased-array tile with a 4x4 grid of copper patches, finned heatsink back, short mount to the mast |
| `lte_modem.bbmodel` | item | Blue module card: gold bay fingers, shielded modem can, SIM tray, stubby whip antenna, link LED |
| `mmwave_modem.bbmodel` | item | Purple module card: same base, a small patch-array antenna tile instead of the whip |
| `sim_card.bbmodel` | item | SIM card with clipped corner, carrier stripe and gold contact pad |
| `card_reader.bbmodel` | facing_card | Card Reader/Writer: graphite casing on a dark plinth, louvred sides, a back label and screws, a card icon on top; the front bezel has the card slot, a small green LCD and an amber LED. Cubes named `card_*` (a SIM sticking out of the slot, a green LED over the amber one) only appear in `card_reader_card`, used when `card=true` |


## ePhone

Built through the Blockbench MCP (64x64 textures at two texels per model pixel, painted procedurally and
embedded), exported with `py -3 scripts/export-radio-models.py ephone_body charging_dock`.

| Project | Kind | Design |
|---|---|---|
| `ephone_body.bbmodel` | item_bewlr:ephone | Graphite phone body with a brushed aluminium frame, a glass front (earpiece, front camera, home bar; the renderer draws the display over x 5-11, y 2.5-12.5 of the north face), camera island, power button and volume rocker. Exported as `models/item/ephone_body.json`; `models/item/ephone.json` is `builtin/entity` with the project's display transforms (screen toward you in either hand) |
| `charging_dock.bbmodel` | facing_active | Graphite base with a rubber top, a back rest with an etched ECM mark, a front lip and gold connector pins; a green lamp (`lit_lamp`) while charging. The docked phone is drawn by the dock's renderer, leaning on the rest |
