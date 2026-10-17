# Playtesting the card system

The build: `evanscomputermod-mc1.21.1-1.0.0.jar` in the repository root (1.21.1, NeoForge 21.1.228; Sable goes in
`mods/` next to it as usual). Everything below is in the creative tab "EvansComputerMod". Design and the full
peripheral reference: [CARDS.md](CARDS.md); the schemes: [NFC_MIX.md](NFC_MIX.md).

## 0. The fast way: the scenarios

Each one builds a working setup in front of you, types the commands and reports `ok` / `FAIL` in chat:

```
/ecm scenario list
/ecm scenario spawn card_print          a printer paints a bank card from a .card design, paying in dye
/ecm scenario spawn card_bank           write a bank chip, PIN, two counter-signed tags, lock it
/ecm scenario spawn card_stripe         write a stripe, read it, clone it onto a blank
/ecm scenario spawn card_nfc            an NFC Reader talks to a card in the chest next to it; 4 blocks away: nothing
/ecm scenario spawn card_reader_module  a Card Reader Module in a Terminal bay
/ecm scenario spawn card_writer         the SIM writer (now `cardtool sim …`)
```

`/ecm scenario spawn <name> manual` builds it and leaves the typing to you; `/ecm scenario commands <name>` prints
the walkthrough.

## 1. Items and blocks

| Item | Recipe (shaped) | Notes |
|---|---|---|
| Bank Card ×4 | `GP / PI` (gold nugget, paper, iron nugget) | chip pad on the front, stripe on the back |
| NFC Card ×4 | `CP / PR` (copper, paper, redstone) | no pad, no stripe |
| Hybrid Card ×2 | `GC / PI` | bank chip + NFC + stripe |
| SIM Card ×4 | unchanged | now a card too: printable, same reader |
| Card Reader/Writer | `IGI / IPI / IRI` | unchanged block, now takes every card |
| Card Printer | `IDI / IPI / IRI` (ink sac in the middle of the top row) | 16 dye slots, card in, printed card out |
| NFC Reader | `CCC / IRI` | a flat pad; lights up while its field is on |
| Card Reader Module | `IGI / IRI` | a bay module: right-click the bay with a card |
| NFC Module | `ICI / IRI` | a bay module (Terminal or ePhone) |

Blocks go **against a side of a Terminal** like any peripheral; `peripherals` on the computer lists them
(`card_reader`, `card_printer`, `nfc_reader`). Right-click a reader with a card to put it in, with an empty hand to
take it out, sneak-right-click (or empty) to open its screen. The **printer is inventory-only**: right-click opens
its screen (card in, card out, sixteen dye slots, one per colour); hoppers and pipes feed the sides and top and
pull from below.

## 2. Print a card

Option A, the **Card Designer** app (on any Terminal desktop / App Store → Installed): New → pick the kind, paint
the front and back (the chip pad and stripe rows are locked), set the palette with the colour picker, fill in
name / number / issuer, the stripe tracks for bank cards, Save. Open the Card Printer next to the computer and
put dyes and a blank card in its slots, then press **Print**: it shows the dye cost, prints, and the card appears in the printer's output
slot (or is pushed into an adjacent chest with Auto Eject on).

Option B, the shell, with a design file (the Designer saves the same format under
`/var/apps/dev.ecm.carddesigner/`):

```
python
>>> import cards
>>> d = cards.Design.new("bank")
>>> d.fill("front", 1); d.text("front", 2, 14, "ALICE", 3)
>>> d.fields["name"] = "ALICE"; d.stripe = ("ALICE", "4000123412341234", "")
>>> d.save("/bank.card"); exit()
cardtool cost bank.card          what dyes it needs (one per palette colour used)
cardtool print bank.card         prints; "printed" when done
```

Things to look at: the card in your hand / an item frame / the inventory shows the painted faces; the back of a
bank card keeps its stripe whatever you paint; a card with no dye of a needed colour is refused with
`no_dye:<colour>`; identical cards stack.

## 3. Bank chip

With a bank card in a Card Reader/Writer:

```
cardtool info
cardtool bank write bank-red 4000123412341234 --random --pin 1234 --pin-required
cardtool bank sign 0001020304050607 1500      refused: PIN required
cardtool bank pin 1234                        pin ok
cardtool bank sign 0001020304050607 1500      ctr 1, tag …
cardtool bank sign 0001020304050607 20        ctr 2 (the counter never repeats)
cardtool bank pin 0000  (three times)         the third locks the chip; sign is refused from then on
cardtool stripe read                          the stripe still reads
```

The key is printed once by `bank write`; it is never readable from the card again (try `/data get` on the item:
the `bank_secret` is there on the server, but a client never receives it).

## 4. Stripes (the skimmer's lesson)

```
cardtool stripe write ALICE 4000123412341234 LOYALTY=42
cardtool stripe read
cardtool eject                     (Auto Eject on: it goes into the adjacent chest)
```

Put a blank in, run the same `stripe write`: the clone is identical (it even stacks with the original). Standing
next to a transmitting amplifier for a few seconds scrambles every stripe you carry (random characters, not blanks).

## 5. NFC

An NFC Reader against the computer; the card can be **anywhere within about a block**: in your hand, in a chest
next to the pad, lying on the floor, in an item frame, in a phone's bay. Four blocks away it is out of the field.

```
cardtool nfc field on
cardtool nfc poll                          uid … kinds nfc
cardtool nfc select <uid>
cardtool nfc auth 1 45434d2d4e46432d464143544f525930   the factory key (ASCII ECM-NFC-FACTORY0) opens sector 1
cardtool nfc write 1 <64 bytes hex>
cardtool nfc read 1
cardtool nfc setkey 1 00112233445566778899aabbccddeeff  from now on the factory key fails
```

Readers at 10 dBm wake a card out to ~2.3 blocks but only hear its reply to ~1.3 (the reply rides on the field).
An iron block between pad and card is a Faraday cage. The same sector commands work through the contacts of a
Card Reader/Writer (`cardtool nfc auth …` with the card in the slot).

## 6. Modules

Terminal: a Module Expansion Card, then a Card Reader Module or NFC Module into a bay. Right-click the bay with a
card to put it in the reader module; sneak-right-click the air holding the loose module to get the card back.
ePhone: the NFC Module in a phone bay reads cards you hold near the phone, and a card in the phone's other bay
(a Card Reader Module) answers pads you walk up to.

## 7. The example programs

They are packages in the Tech Village app repository (App Store → Discover, or `pkg search card`): **bankd**
(issuer: `pkg run dev.ecm.bankd issue alice 4000123412341234 --pin 1234`, `… serve 7000`), **pos**
(`pkg run dev.ecm.pos <bankd-ip>:7000 30`: nonce → the card signs → bankd verifies tag and counter), **doord**
(an NFC door; `--uid-only` is the deliberately bad mode a cloned UID fools), **skimmer** (stripe cloning),
**rx_nfc** (SDR burst sniffer on 13.56 MHz + raw-frame probes). Without a village repository the sources are in
`examples/cards/` in the repo: copy a `main.py` onto a computer and run it with `python`.

## Known gaps

- No `card_phone_tap` scenario yet (phone + NFC module + pos + bankd is a manual test).
- The stripe hazard has no scenario (needs an SDR + amplifier chain); it is unit-tested.
- Bay cartridges for the two modules use the generic look.
