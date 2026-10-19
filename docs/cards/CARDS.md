# Cards: bank cards, NFC cards, readers, printer and designer

Status: design (2026-10-16). Nothing built yet. Generalises the smart card and Card Reader/Writer that
shipped with Cellular v2 ([CELLULAR_V2.md](../gui/CELLULAR_V2.md), "As built: smart card and card writer"),
whose spec already said "reusable for keycards later". 1.21.1 only, like the `card` package today.

## Why

The mod adds **cards as hardware** and nothing that uses them. A bank, a door lock, a loyalty scheme, a
transit gate are programs players write against the peripherals here, the way cell networks are programs
over the framed radio. What the mod guarantees is physical: a card's secret never leaves the card, a
contact reader needs the card in its slot, a contactless reader needs the card in its field, a stripe is
only text, and a printer only paints.

Three deliberately different trust tiers live on the same chassis, so programs have real choices to get
wrong:

| Tier | Interface | What it protects against | What it doesn't |
|---|---|---|---|
| **Magnetic stripe** | swipe, contact | nothing (plain text) | anyone with a reader + writer clones it; a strong field scrambles it |
| **Bank chip** | contact (reader slot) | forgery and replay (keyed, counted) | theft of the card itself (a PIN gate helps) |
| **NFC applet** | near-field radio | forgery (per-sector keys) | sniffing by an SDR in the field; readers that trust the UID alone |

The security schemes are **toys by design**. They must hold up inside the game (no key extraction, no
replay) while sharing nothing with EMV, ISO 14443, DESFire, Crypto-1 or any other real protocol, so nothing
learned here transfers to real cards and nothing real transfers here.

## Architecture

```
 Card Designer app           Card Printer (block)              Card Reader/Writer (block) / reader module
 ┌───────────────────┐ print ┌──────────────────────────┐      ┌──────────────────────────────────────┐
 │ pixel faces       │ ────► │ blank in, printed out    │      │ chip: challenge/response, PIN, write │
 │ palette, fields   │       │ dyes, stripe encoding    │      │ stripe: read/write tracks            │
 │ .card files       │       │ peripheral card_printer  │      │ peripheral card_reader               │
 └───────────────────┘       └──────────────────────────┘      └──────────────────────────────────────┘
                                                                                 ▲ contact
            near-field radio (passive card endpoints)                            │
 ┌──────────────────────────┐   field   ┌───────────────────────┐   ┌────────────┴───────────┐
 │ NFC Reader (block/module)│ ~~~~~~~~~ │ card in a hand, chest,│   │ a card item (one       │
 │ energises, polls, talks  │ ◄~~~~~~~~ │ hopper, phone, floor… │   │ chassis, 1+ applets)   │
 │ peripheral nfc_reader    │ backscatter└───────────────────────┘   └────────────────────────┘
 └──────────────────────────┘
          ▲
   player programs: bankd, doord, skimmer, transit gate, … (Python / Lua / Rust, via `cards` module)
```

## Primitives

| Primitive | Status | Notes |
|---|---|---|
| Card item chassis, applets, redacted secrets | **generalise** | From `card/SmartSim`, `SimSecret`, `SimSecretGuard`. SIMs become an applet on the chassis. |
| Card Reader/Writer block + `card_reader` peripheral | **generalise** | Keeps the held/released/auto-eject life cycle; dispatches to the card's applets; gains stripe methods. |
| Pixel faces + per-stack renderer | new | Palette-indexed faces in a synced component; `DynamicTexture` behind a BEWLR, the ePhone's `PhoneItemRenderer` pattern. |
| Card Printer block + `card_printer` peripheral | new | Graphics only: paints faces, encodes the stripe, consumes dye. Never touches chip or sector data. |
| Near-field band + path model | new (radio) | `Band.NEAR_FIELD`, inductive-coupling loss, no sky/ground wave, conductive blocks shield. |
| Passive endpoint (energise → answer) | new (radio) | `PassiveEndpoint`: no transmitter of its own; replies at field strength minus a backscatter loss. General: RFID tags, passive sensors later. |
| Card endpoint tracker | new | Reader-driven: cards in any inventory, item entity, hand or phone within a field get an endpoint while a field is on. |
| NFC Reader block + NFC reader module | new | A `RadioEndpoint` on the near-field band that energises and polls; peripheral `nfc_reader`. The module works in Terminal bays and ePhone bays. |
| Card reader bay module | new | Contact reader in a bay (`IModuleItemTarget`: right-click a card into it), peripheral `card_reader`, same methods as the block. |
| `ecm-ui` colour picker + pixel grid widgets | new (UI) | Reusable widgets with Python and Lua bindings. |
| Card Designer app | new | System app `dev.ecm.carddesigner` (Python). |
| `cards` module (Python + Lua), `cardtool` (Rust) | new | Thin wrappers over the peripherals; `simtool` folds into `cardtool sim …`. |
| Module bays, ePhone bays, peripherals, device files, app platform, App Store | exists | Unchanged. |

## The card item

One item, `evanscomputermod:card`, in the item tag `evanscomputermod:cards`. Its **kind** fixes the chassis
(size, face dimensions, which fixed features the renderer draws) and which applets it may carry:

| Kind | Face size (texels) | Fixed features | Applets allowed | Stripe |
|---|---|---|---|---|
| `bank` | 32 × 20 | chip contact pad (front), stripe (back) | `bank` | yes |
| `nfc` | 32 × 20 | none | `nfc` | no |
| `hybrid` | 32 × 20 | chip pad, stripe | `bank`, `nfc` | yes |
| `sim` | 13 × 8 | contact pad | `sim` | no |

Blanks are craftable (`blank_card` recipes give a kind-tagged card with no applet data, no faces, no stripe
data). A blank is "blank" per applet: a printed card can still be an unwritten bank card.

### Components

| Component | Type | Saved | Sent to clients | Written by |
|---|---|---|---|---|
| `card_kind` | enum | yes | yes | crafting |
| `card_faces` | `CardFaces(palette[≤16] RGB, front[], back[])`, 4-bit indices, row-major, 0 = palette entry 0 | yes | yes (hash-cached textures) | printer |
| `card_print` | `Map<String,String>` public fields (`name`, `number`, `issuer`, `label`, free keys), each ≤ 64 chars, ≤ 8 keys | yes | yes (tooltip) | printer, reader `write_fields` |
| `card_stripe` | `Stripe(t1 ≤ 64, t2 ≤ 32, t3 ≤ 96 chars, corrupted: bool)` | yes | yes | printer, reader/writer |
| `card_uid` | 8 random bytes, set when the card is first written by any applet | yes | yes | first applet write |
| `bank_applet` | `BankPublic(issuer, pan, locked: bool, ctr: long)` | yes | yes | reader/writer |
| `bank_secret` | `BankSecret(K[32], pinHash[32])` | yes | **no** (redacted stream codec, `SimSecret` pattern) | reader/writer, once |
| `nfc_applet` | `NfcPublic(sectors: 8 × {written: bool})` | yes | yes | reader |
| `nfc_secret` | `NfcSecret(keys 8 × 16 B, data 8 × 64 B)` | yes | **no** (redacted) | reader, per sector |
| `sim_card`, `sim_secret` | unchanged | yes | yes / **no** | as today |

Secrets reuse the whole existing machinery: the redacted stream codec, `SimSecretGuard` → renamed
`CardSecretGuard` (restores every `*_secret` component of a creative-moved card by `card_uid`), the reader
block entity's empty update tag, `ModemSimStore` → `CardStore` for cards kept inside other items (phone
bays, modem slots).

Sector *data* on the NFC card is in the secret component on purpose: the only way to read it is through an
authenticated sector session on a reader, so a client (or `/data get`) learns nothing.

### Applets (Java, `card/applet/*`)

`CardApplet` is the SmartSim shape generalised: pure logic over a `Storage` of components. `CardItem.applets(stack)`
returns the applets a card carries; `BankApplet`, `NfcApplet`, `SimApplet` (today's `SmartSim`). Readers and the
`card` peripheral dispatch by applet name. Applet methods are the card's silicon: they are the only code that
sees a secret, they never return one, and they are unit-tested as plain Java.

## Faces: pixel art

- Fixed resolution per kind (table above). A face is a palette-indexed bitmap; the palette is per card,
  ≤ 16 RGB entries; 32×20×2 faces + palette ≈ 690 bytes.
- **Masks.** The renderer composites the kind's fixed features *over* the face data: the stripe rectangle
  (back, rows 2–5 full width) and the chip pad (front, a 6×5 rectangle at the left). The Designer greys and
  refuses those texels; a program that writes face bytes there still never sees them painted. A stripe
  with `corrupted = true` gets a scratched variant texture.
- **Rendering.** `CardItemRenderer` (BEWLR via `IClientItemExtensions`, like `phone/client/PhoneItemRenderer`):
  the Blockbench chassis for the kind (edges, pad, stripe) with a static texture; front and back quads sample
  a `DynamicTexture` built from `card_faces`. Textures are cached by the faces' hash (a bank's 200 cards share
  one), evicted LRU. The inventory icon is the front face flat; in hand and in item frames it is the slab.
  Blank faces render as the kind's default (off-white, with features).
- **Models.** One `.bbmodel` per kind in `models/` (`card_bank`, `card_nfc`, `card_hybrid`; `sim_card` already
  exists and is re-exported with named front/back faces), made with the Blockbench MCP, with previews, per
  AGENTS.md.

## Magnetic stripe (bank, hybrid)

Three tracks of plain text with our own limits (64 / 32 / 96 characters, printable ASCII), no auth, no
secret. Readable by anything that can swipe; writable by the printer and the Card Reader/Writer.

- **Clonable**: `read_stripe()` on one card, `write_stripe()` on a blank — a skimmer is a ten-line program.
- **Erasable**: `StripeHazard` (server, cheap): when a card with a stripe is inside a strong field — a running
  amplifier, a microwave link's feed, the existing `radio/power` hazard volumes — for more than 2 s, each
  track is corrupted (random printable characters at random positions, `corrupted = true`). Programs that
  trust a stripe must validate it; a bank program sees garbage, not an empty card.
- The stripe has no radio surface: possession is the only access path. Shielding is irrelevant to it.

## Bank chip ("keyed ledger response")

Contact only (reader slot, or the `card` peripheral through a bay). Toy scheme, version 1:

```
K    = 32-byte card key (bank_secret), CTR = 64-bit monotonic counter (public, bank_applet)
NONCE = 8 bytes from the reader, AMOUNT = 8-byte big-endian integer (0 when unused), labels ASCII, || concat
TAG  = HMAC-SHA256(K, "ECM-BANK-TAG" || NONCE || CTR || AMOUNT)[0..16)
```

- `sign(nonce, amount)` increments `CTR` **before** computing TAG and returns `{ctr, tag}`. The issuer verifies
  with its copy of K and rejects any CTR it has seen or that moved backwards. No certificates, no cryptogram
  types, no application selection: nothing EMV.
- **PIN**: `pin_hash = SHA-256("ECM-BANK-PIN" || card_uid || pin)` (4–8 digits). `verify_pin(pin)` opens a
  session (until the card leaves the reader); `sign` requires an open session when the issuer set
  `pin_required`. Three failures set `locked = true`: the chip answers nothing more; the stripe still reads.
  `unlock(puk)` is deliberately absent: a locked card is reissued. Each failed try is also an event.
- `write_bank(issuer, pan, key_hex, pin, pin_required)` once, on a card whose bank applet is blank; PAN 8–19
  digits, issuer `[a-z0-9_-]{1,24}`. Nothing reads K back; `respond` on a client-side copy answers nothing.

## NFC applet ("sector lattice")

8 sectors × 64 bytes, each with a 16-byte key. Blanks ship with every key = the published factory key
`ECM-NFC-FACTORY-00` (padded) so a card works out of the box and *rekeying is the program's job*.

```
Auth(sector i):  reader → POLL;  card → UID (always, no auth)
                 reader → AUTH i, R1 (8 B);  card → R2 (8 B), M1 = Mix(S_i, R1 || R2)
                 reader → M2 = Mix(S_i, R2 || R1);  card checks M2 → session open for sector i
Mix(key, x):     8 rounds of add-rotate-xor over (x expanded to 16 B) with key words; 16-byte result.
```

`Mix` is a toy ARX construction specified in `docs/cards/NFC_MIX.md` with test vectors (so `bankd`-style
programs in Rust/Python/Lua can implement the reader side; a Python reference ships in the `cards` module).
It is not Crypto-1 and not AES. Per session: `read(i)` → 64 bytes, `write(i, data)`, `set_key(i, new_key)`.
Frames are visible to any SDR tuned to the near-field band inside the field; a replay fails because R2 is
fresh, but a sniffed session's data is exposed, and a reader that only trusts the UID is trivially fooled.
Those are the intended lessons.

## Near-field radio (new primitives in `radio/`)

### Band and path model

- `Band.NEAR_FIELD(13.553e6, 13.567e6)`, checked first in `Band.of` (a carve-out inside HF). The medium is
  sharded per band, so it gets its own shard and its own path model.
- `phys/NearField.lossDb(d, f)`: inductive coupling, `L = L0 + 60·log10(max(d, 0.25) / 0.25)` with L0 chosen
  so a reader at the spec power closes the link at ≈1.5 blocks and fails by ≈3. No ground wave, no skywave,
  no fading beyond the PER roll. Conductive blocks on the path (iron, copper, gold, netherite blocks,
  iron doors/bars/trapdoors; a `#evanscomputermod:nf_shield` block tag) add 40 dB each: an iron vault is an
  RFID-blocking wallet. Uses the existing `PathTracer` voxel sampling.

### Passive endpoints

`PassiveEndpoint extends RadioEndpoint` (in `radio/api`):

- `maxTxPowerDbm()` is not its own: it returns the power of the field currently energising it minus
  `backscatterLossDb()` (default 20), or `-∞` when not energised.
- `wakeThresholdDbm()`: an `ENERGY` reception at or above it (on its tuned channel) sets `energisedUntil =
  now + hold` (hold = 2 ticks) and records the field power; `energised()` is true until then.
- `transmit` from a non-energised passive endpoint is dropped by the medium (and counted, for tests).
- Reply range is therefore bounded by the reader's field: a strong illuminator far away still only gets a
  reply if the return link closes at field power − 20 dB. Sniffing a legitimate session a few blocks away
  stays possible; interrogating across the base does not.

Unit tests in `radio/phys` and `radio/medium` cover: loss curve, shield attenuation, wake/hold, reply
power derivation, dropped transmit when cold.

### Anticollision

`POLL` from the reader; every energised card answers its UID in one of 16 slots chosen from its UID bits;
overlapping replies are just PER losses in the medium. Readers retry `POLL` a few times and report every
UID heard; `select(uid)` makes further commands address one card. Toy by construction.

### Card endpoint tracker (`card/nfc/CardEndpointTracker`, server)

Cards answer **anywhere within range**: hand, hotbar, inventory, chests and every item-handler block
entity, item entities on the ground, item frames, ePhone bays, Terminal module slots. The cost is paid only
while a field is on:

- An NFC reader with polling enabled registers a *field volume* (its pose, radius 3 blocks — generous
  so the path model, not the volume, sets the range).
- Each tick with any field on, the tracker scans field volumes for: item entities, block entities with
  `Capabilities.ItemHandler`, players (inventory + offhand + phone bays), minecarts with chests. Each card
  found gets (or refreshes) a `CardEndpoint` (passive, pose = container's centre or holder's hand, `speedMps`
  from the holder). Endpoints not seen for 10 ticks are unregistered.
- Cards nested in other items (a card inside a phone bay inside a chest) are found through `CardStore`.
- No field anywhere → the tracker does nothing. Budget: a GameTest asserts the scan of a 3-block cube with
  20 chests stays under 1 ms (cold).

## Hardware

### Card Reader/Writer block (`card_reader`, generalised)

Everything that exists (one slot, hand or hopper in, held/released, Auto Eject, comparator, screen) stays;
"Auto Eject Sim" is renamed **Auto Eject**. Right-click with a card inserts; empty-hand right-click ejects;
sneak or no card opens the screen; the screen also shows the card's faces (same renderer). Peripheral
`card_reader` (server thread):

| Method | Returns |
|---|---|
| `present()`, `eject()`, `status()` | as today; `status.state` is now `empty` / `card` and `status.applets` lists them |
| `identity()` | `{kind, uid?, applets: [...], print: {...}, bank?: {issuer, pan, locked, ctr, blank}, nfc?: {sectors}, sim?: {imsi, carrier, blank}}` |
| `read_stripe()` | `{t1, t2, t3, corrupted}` or nil without a stripe |
| `write_stripe(t1, t2, t3)` | writes and clears `corrupted` |
| `write_fields({...})` | sets `card_print` (the printer does the same) |
| **bank** | `write_bank(issuer, pan, key_hex, pin, pin_required)`, `verify_pin(pin)`, `sign(nonce_hex, amount)` → `{ctr, tag}`, `bank_blank()` |
| **nfc** (contact fallback, same applet) | `nfc_auth(sector, key_hex)`, `nfc_read(sector)`, `nfc_write(sector, hex)`, `nfc_set_key(sector, key_hex)` |
| **sim** | `write(imsi, carrier, key_hex)`, `is_blank()`, `respond(challenge_hex)` unchanged (deprecated aliases of `write_sim`, `sim_blank`, `sim_respond`) |

Events: `card_inserted(attachment, kind, applets)`, `card_removed(attachment)`, `pin_failed(attachment, tries_left)`,
`card_locked(attachment)`.

### Card reader module (`card_reader_module`)

The same reader in a bay: `IComputerModuleItem` + `IComputerModule` + `IModuleItemTarget` (right-click a card
onto the bay slot to insert, sneak-right-click to take it out), `ModuleSlotVisual.CARD_READER` cartridge
models. Peripheral type `card_reader`, identical methods, so programs don't care which. State: the card's
public components in the module's custom data; secrets through `CardStore` (the modem/SIM pattern).

### NFC Reader block (`nfc_reader`) and NFC module (`nfc_module`)

A `RadioEndpoint` on `NEAR_FIELD` (block: a small pad, one coil; module: the same silicon in a bay, pose from
the host — a phone's module follows the player). Peripheral `nfc_reader`:

| Method | Meaning |
|---|---|
| `set_field(on)`, `field()` | energise (emits `ENERGY` every tick at `tx_dbm`), default off |
| `set_power(dbm)` | 0 … 10 dBm (block), 0 … 6 dBm (module) |
| `poll()` | `[{uid, kinds}]` heard in up to 4 rounds |
| `select(uid)` | address one card |
| `auth(sector, key_hex)`, `read(sector)`, `write(sector, hex)`, `set_key(sector, key_hex)` | the sector lattice, over the air |
| `raw(hex)` | send an arbitrary frame and return the reply (for players writing their own protocols) |
| `rssi()` | last reply's RSSI |

Events: `nfc_seen(attachment, uid)` (new UID in the field), `nfc_gone(attachment, uid)` (10 ticks without
reply), `nfc_error(attachment, reason)` (collision, auth failure).

Air frames (`docs/cards/NFC_AIR.md`): a tiny TLV (`cmd`, `len`, body) at "NF-106" modulation (106 kb/s), so
an SDR program on the same band can decode them with `ecm-radio`'s framed decoder; `rx_nfc` ships as an
example sniffer.

### `card` peripheral (through bays and phone slots)

Today's `sim` peripheral generalised: a card held inside a device (phone tray, a reader module) exposes its
applets to that device's programs — this is how a phone's `celld` reaches its SIM, and how a wallet app on a
phone could sign with a bank card that lives in the phone's second bay. Methods mirror the reader's applet
methods. `sim` stays as an alias type.

### Card Printer block (`card_printer`)

- Slots: blank/card in (any card; faces can be reprinted), card out, 16 dye slots (one per vanilla dye
  colour, any count). Hopper/pipe in on the sides and top, out on the bottom; comparator = cards waiting.
- Screen: the slots, a preview of the card (both faces, the renderer), progress.
- Peripheral `card_printer`:

| Method | Meaning |
|---|---|
| `present()`, `status()` | `{present, kind, printing, progress, dyes: {white: 3, …}}` |
| `print(design)` | `design = {palette: [...], front: hex, back: hex, fields: {...}, stripe: {t1, t2, t3}?}`; validates sizes against the kind; **consumes one dye per palette colour used**, matched to the nearest vanilla dye by hue (a 16-colour palette costs at most 16 dyes; "white" counts); errors `no_dye:<colour>` list what's missing; prints over 2 s, then releases (Auto Eject as the reader) |
| `cost(design)` | the dye list without printing |
| `eject()` | as the reader |

The printer never reads or writes `*_secret`, `bank_applet`, `nfc_applet` or `sim_*`; a locked or
written card reprints fine. A blank stays blank per applet after printing. `card_uid` is not set by printing.

Events: `card_inserted`, `card_removed`, `print_done(attachment)`, `print_failed(attachment, reason)`.

## Programs and apps (Rust/Python/Lua — everything above the silicon)

- **`cards` module** (Python `cards.py`, Lua twin, parity-tested): `Reader.find()`, `card = reader.card()`,
  `card.bank.sign(nonce, amount)`, `card.stripe.read()`, `Nfc.find().poll()`, `nfc.session(uid, sector, key)`,
  `Printer.find().print(Design.load(path))`, `Design` (load/save `.card` JSON, `set_pixel`, `fill`,
  `text(x, y, s)` with a 3×5 font), `bank.verify_tag(K, nonce, ctr, amount, tag)`, `nfc.mix(key, x)` reference.
- **`cardtool`** (Rust, replaces `simtool`): `cardtool info | stripe read|write | bank write|pin|sign |
  nfc poll|auth|read|write|setkey | print <file> | eject | wait`; `cardtool sim …` keeps simtool's verbs.
- **Card Designer** (`dev.ecm.carddesigner`, Python, ecm-ui): front/back tabs over a `pixel_grid` widget
  (integer zoom, pointer → texel, masked regions greyed), tools pencil / fill / eyedropper / line /
  mirror-H, a 16-swatch palette with the new `color_picker` widget (hue strip + SV square + hex field),
  fields editor (name, number, issuer, label), stripe tracks editor (bank/hybrid), New (choose kind),
  Open / Save `.card` under `/var/apps/dev.ecm.carddesigner/`, **Print** (finds a `card_printer`, shows
  `cost`, prints) and **Export** when no printer is attached. Permissions: `files:/var/apps/dev.ecm.carddesigner`,
  `peripherals:card_printer`.
- **`.card` file**: `{"kind": "bank", "palette": [[r,g,b], …], "front": "<base64 4-bit>", "back": "…",
  "fields": {…}, "stripe": {…}}` — plain so programs can stamp templates (a bank writes the customer's name
  onto its template and prints).
- **ecm-ui widgets** (built, [UI_PROTOCOL.md "Card widgets"](../gui/UI_PROTOCOL.md#card-widgets)): `pixel_grid`
  (indexed bitmap drawn as rect runs, zoom or auto-fit, pencil / fill / eyedropper / line, `mirror_h`, mask,
  `change` with the changed texels, bitmap read back from the widget's state) and `color_picker` (hue strip,
  SV square, hex field; `change` with `0xRRGGBB`), in `rust/crates/ecm-ui` with Python (`ui.PixelGrid`,
  `ui.ColorPicker`) and Lua twins; golden `card_widgets`.
- **Example apps in the village repository** (store packages, so players have templates): `bankd` (issuer:
  subscriber file, writes cards through a reader, verifies `sign`, keeps balances), `pos` (point of sale:
  reader + NFC, charges `bankd` over TCP), `doord` (NFC door: allow-list by sector 1 data — and a
  deliberately bad `doord --uid-only` mode), `skimmer` (reads a stripe, writes it to the next blank),
  `rx_nfc` (SDR sniffer for the near-field band).

Permissions (`ProcessSandbox`): `peripherals:card_reader`, `peripherals:nfc_reader`, `peripherals:card_printer`;
the `card` peripheral maps to `cellular` for `sim` applets and `peripherals:card` otherwise; the near-field
SDR path is `radio` like any band.

## SIM retrofit

- `sim_card` item → `card` with `card_kind = sim` (item id alias + recipe; existing stacks are remapped by a
  simple item-id migration in `CardContent`, components untouched).
- `SmartSim` → `SimApplet`; `SimCardItem.card/sim(stack)` → `CardItem.applet(stack, SimApplet.class)`; `SimSecretGuard`
  → `CardSecretGuard`; `ModemSimStore` → `CardStore`; `SimProvider` → `CardProvider`. The `sim` peripheral and
  its methods are unchanged, so `celld`, `cored`, the phone tray and the modem slot keep working.
- SIMs get pixel faces (13×8) and can be printed; `card_writer` scenario and `ecm_cellular`/`ecm_phone`
  GameTests are the regression gate.

## Scenarios (`/ecm scenario spawn …`, auto/manual/fast; real player steps)

| Scenario | Steps and checks |
|---|---|
| `card_print` | computer + printer + chest; a blank bank card, dyes in the printer; `cardtool print bank.card` → the card lands in the chest with those faces and fields, dye counts dropped by the palette's colours, a second print with a missing dye fails with `no_dye:red`; the client render check shows both faces (and the stripe over painted texels) |
| `card_bank` | printer → reader/writer: `cardtool bank write` sets issuer/PAN/key/PIN; `cardtool bank sign` twice gives rising `ctr` and tags that the server's HMAC matches; wrong PIN ×3 locks (event, `sign` refused), stripe still reads; the card's network encoding has no key |
| `card_stripe` | reader writes tracks; a second reader + blank clones them; the clone reads identical; a card left 3 s next to a running 1 kW amplifier comes back `corrupted` with changed tracks |
| `card_nfc` | NFC reader block on a computer; a card in hand at 1 block: `poll` sees it, `auth` with the factory key, `write` sector 1, `set_key`; old key fails; at 4 blocks `poll` is empty; same card inside a chest at 1 block is seen, inside an iron-block box it is not; an SDR with `rx_nfc` 2 blocks away logs the session's frames |
| `card_phone_tap` | an NFC module in an ePhone bay, `pos` on a Terminal with an NFC reader block; the player walks up with the phone (card in the other bay) → `pos` charges `bankd` over the network |
| `card_reader_module` | reader module in a Terminal bay: right-click a card onto it, `cardtool info`, sneak-right-click out |
| `card_writer` | existing SIM scenario, unchanged except the item is the `sim`-kind card |

## Tests

| Layer | Test |
|---|---|
| JUnit (plain Java) | `BankAppletTest` (vectors for TAG, PIN hash, counter, lock), `NfcAppletTest` (`Mix` vectors, 3-pass auth, wrong M2), `CardFacesTest` (codec, size validation, mask), `StripeTest` (limits, corruption), `CardSecretTest` (every `*_secret` has an empty network encoding), `NearFieldTest` (loss curve, shields), `PassiveEndpointTest` (wake/hold, reply power, cold transmit dropped) |
| Rust | `ecm-ui` widget goldens; `cards`/`cardtool` unit tests; `nfc::mix` vectors identical to Java's |
| Python/Lua | parity test extended (`cards` module); `nfc.mix` and `bank.verify_tag` against the shared vectors |
| GameTest | `ecm_cards`: every scenario above in auto mode; `tracker_budget`; `creative_slot_keeps_key` extended to bank and nfc secrets |
| Client | `cards_closeup`: printed bank/nfc/hybrid/sim cards in hand, in an item frame, in the reader and printer screens; corrupted stripe variant |
| Regression | `ecm_cellular`, `ecm_phone`, `card_writer` |

Commands follow TESTING.md (`scripts\Test.ps1 -JUnit …`, `-GameTests ecm_cards`, `-ClientChecks -ClientSuite cards`).

## Phases and done-when

1. **Chassis + SIM retrofit.** `CardItem`, kinds, applet interface, `SimApplet`, `CardSecretGuard`, `CardStore`,
   item migration; Card Reader/Writer dispatches by applet; `cardtool sim …`. Done when `card_writer`,
   `ecm_cellular` and `ecm_phone` pass unchanged in behaviour.
2. **Faces, printer, dye.** `card_faces`, renderer, three new Blockbench models + re-exported SIM, printer block,
   `card_printer`, `.card` format, `cardtool print`. Done when `card_print` passes with a render check.
3. **Stripe + bank chip.** Components, reader methods, `BankApplet`, PIN/lock, `StripeHazard`, reader module.
   Done when `card_bank`, `card_stripe`, `card_reader_module` pass.
4. **Near-field radio.** `Band.NEAR_FIELD`, `NearField` loss + shields, `PassiveEndpoint`, medium support,
   unit tests. Done when the radio tests pass and an SDR scenario hears a frame at 1 block and not at 4.
5. **NFC.** `NfcApplet` + `Mix`, `CardEndpointTracker`, NFC reader block + module, air frames, `rx_nfc`.
   Done when `card_nfc` and `card_phone_tap` pass.
6. **Designer + widgets + examples.** `pixel_grid`, `color_picker`, Card Designer app, `cards` module (Py+Lua),
   `bankd` / `pos` / `doord` / `skimmer` packages. Done when the Designer prints a card end-to-end in a
   scenario and the parity test passes.

Phases 2–3 and 4 can run as parallel lanes after phase 1; 5 needs 1 and 4; 6 needs 2 and 5.

## Open questions

- Face resolution: 32×20 is a guess at "readable on an in-game screen"; confirm with the first render.
- Should an NFC card in a *player's* inventory answer a reader through a wall? The spec says yes (pickpocketing
  is a feature; shield with iron). Revisit if it reads as griefing in play.
- Dye-to-palette matching by nearest hue is lossy (a pastel and a saturated red both cost "red"); fine for v1.
- Whether the Card Designer should also drive the reader (write fields) — kept out: the Designer is a
  printer client only.

## As built (2026-10-16, branch `feature/cards`)

Everything above is built on 1.21.1 with these differences and notes:

- **One item per kind**, not one item with a `card_kind` component: `bank_card`, `nfc_card`, `hybrid_card` (new,
  `CardContent`) and `sim_card` (`SimCardItem extends CardItem`, kind SIM, still registered by `CellularContent` with its
  `sim_card` / `sim_secret` components). No migration was needed. The item tag `evanscomputermod:cards` lists all four.
  The SIM's faces are portrait, 8×13 texels, to match its existing model.
- **Components** (`CardContent`): `card_faces`, `card_print`, `card_stripe`, `card_uid` (a long), `bank_applet`,
  `bank_secret`, `nfc_applet`, `nfc_secret`; every `*_secret` uses `Redacted.streamCodec`. `SimSecretGuard` became
  `CardSecretGuard` (restores all three secrets by UID or SIM identity). `ModemSimStore` is the server-side store for
  cards inside modules (the card reader module uses it too); `CardStore` was not introduced.
- **Applets** (`card/applet`): `BankApplet`, `NfcApplet` (+ `NfcMix`), `SmartSim` stays the SIM applet.
  Vectors: [NFC_MIX.md](NFC_MIX.md) (`BankAppletTest`, `NfcAppletTest`; the same vectors in `ecm-cards`, Python, Lua).
- **Reader**: `CardReaderPeripheral` works over a `CardHolder` (the block entity or `CardReaderModule`); the PIN session
  and open NFC sectors are a `CardSession` in the peripheral, gone when the card leaves. "Auto Eject Sim" is "Auto Eject".
- **Near field**: `Band.NEAR_FIELD` (13.553–13.567 MHz, checked first in `Band.of`), `NearField.lossDb` (L0 = −3 dB,
  60 dB/decade from 0.25 m), `PassiveEndpoint` (energise → reply at field − 20 dB; a cold transmit is dropped and
  counted), thermal noise instead of the HF atmospheric floor on that shard, no fading/skywave. The air channel is
  13.56 MHz / **848 kHz** (the subcarrier sidebands), so a 106 kb/s frame has processing gain; POLL slots are 1 ms
  apart (longer than a POLL reply). The `#evanscomputermod:nf_shield` block tag is **not** implemented: shielding is the
  medium's existing metal attenuation (iron/copper/gold 80 dB per block), which the unit tests show is a Faraday cage.
- **Ranges** (reader at 10 dBm, the block's default and maximum; the module's is 6 dBm): a card wakes to about 2.3
  blocks and its reply is readable to about 1.3. `NfcRadio` defaults to full power; `set_power` turns it down.
- **Tracker**: `CardEndpointTracker` scans item entities, item frames, container entities, item-handler block entities
  and players (inventory, offhand, phone bays through `ModemSimStore`, the phone's SIM tray) within 3 blocks of each
  live field; a FakePlayer (the scenarios' player) is not in `level.players()`, so the hand case is only covered by real
  play, and the `card_nfc` scenario uses a chest and a dropped item instead.
- **Stripe hazard**: `StripeHazard` scrambles stripes within 2.5 blocks of a transmitting amplifier (3 s); microwave
  radios are not a source. It has no scenario yet (it needs a whole SDR + amplifier chain); `Stripe.corrupt` is unit-tested.
- **Printer**: loaded only through its inventory (the screen's slots, hoppers, pipes) — no right-click loading;
  16 dye slots, one per vanilla colour; `cost` matches palette entries to the nearest dye by RGB distance and charges
  one dye per colour the design uses (masked texels don't count); a job takes 40 ticks.
- **Card bodies** are a plain slab: the chip pad and the stripe are drawn only in the face texture
  (`CardFaceTexture` composites them over the masked texels), so they appear once.
- **Models**: `models/card_{bank,nfc,hybrid,sim}_body.bbmodel` (drawn by `CardItemRenderer` + `CardFaceTexture`),
  `card_printer.bbmodel` (`lit_*` cubes when printing), `nfc_reader.bbmodel` (`field_*` cubes when the field is on),
  exported with `py -3 scripts/export-radio-models.py`. The two bay modules use the **generic cartridge**: a
  `ModuleSlotVisual` value multiplies the Terminal's block states by (n+1)⁴/n⁴ (four slot properties), and two
  more took model baking out of the client's 2 GB heap.
- **Not built**: the `card_phone_tap` scenario and the SDR sniffer step of `card_nfc`.
- **Removed after shipping**: the Card Designer system app (`dev.ecm.carddesigner`) and the `pos`, `doord`, `skimmer`
  and `rx_nfc` example packages. The design text above still describes them; it is not what ships. Designs are made
  with the `cards` module and printed with `cardtool print`; `SystemApps.RETIRED` takes an installed Card Designer off
  a computer (its `/var/apps/dev.ecm.carddesigner` files stay). The `pixel_grid` and `color_picker` widgets stay in
  ecm-ui (golden `card_widgets`).
- **Example package** (`examples/cards/*`, built into the Tech Village repository by `scripts/stage-wasm.sh` next to
  Discord and Music): `dev.ecm.bankd` (issuer: `issue`, `serve <port>`, replay-safe counter checks).
  `python`'s `sys.argv` is now set like CPython's (it was empty), which it needs.
- **Render check**: `scripts\Test.ps1 -ClientChecks -ClientSuite cards` (`CardsServerChecks` / `CardsClientChecks`):
  the three blocks, a printed bank card in the hand (the renderer must report a bank card's faces), four framed
  cards, the Card Printer screen opened by right-clicking the block, and the inventory icons; screenshots under
  `artifacts/cards-render-*/screenshots/`.
- **Tests**: JUnit `BankAppletTest`, `NfcAppletTest`, `CardFacesTest`, `NfcAirTest`, `NfcRadioTest` (+ lane R's
  `NearFieldTest`, `PassiveEndpointTest`); Rust `ecm-cards` (14), `ecm-ui` (`tests/cards.rs`), `python` (`cards_tests`,
  `cards_apps_tests`), `lua`; GameTests `ecm_cards`; goldens `card_widgets`; the radio client suite frames printed cards.
- **Reader display**: the Card Reader/Writer draws the card actually in its slot (`CardReaderRenderer`, the item's own body
  and painted faces) from a client-safe copy in its update tag (item, faces, stripe-corrupted flag; never applets or
  secrets); the block model's fixed SIM cube is gone. Right-clicking a card into a reader (block or bay module) consumes
  it in creative too: a card is handed over, not placed. The reader and printer are `noOcclusion` partial blocks with
  matching shapes, so a Terminal they stand against keeps its faces.
