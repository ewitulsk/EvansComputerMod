# Bluetooth and AirPods

Every ePhone has a Bluetooth radio. AirPods pair with it and connect to it over
the radio medium: their advertising, the pairing, and the audio both ways are
real 2.4 GHz frames, so range, walls and other 2.4 GHz traffic (Wi-Fi,
controllers) apply. While connected AirPods are in your ears, the phone's sound
plays in them (only you hear it) and its microphone is theirs: the phone's
**headphones layer**.

## Items

| Item | What it does |
|---|---|
| **AirPods** | Wireless earbuds with microphones. Right-click to put them in your ears (the head slot; they show at your ears). Battery: about three hours of use connected. Recipe: quartz, quartz over iron nugget, redstone, iron nugget. |
| **AirPods Case** | Right-click: open or close the lid (it closes by itself after a minute); the AirPods inside are in **pairing mode** while it is open. Sneak + right-click: take the AirPods out and into your ears (into your inventory if your head slot is taken); with the case empty, put the AirPods you wear back in. In an inventory, click AirPods onto the case to put them in. AirPods in the case charge. Recipe: quartz, iron ingot, quartz over quartz, redstone, quartz. |

## Pairing and connecting

1. Open the case's lid near your phone.
2. On the phone: **Settings > Bluetooth** (Sections, Bluetooth). Under OTHER DEVICES
   "&lt;you&gt;'s AirPods" shows **ready to pair**. Tap **Pair**.
3. They pair (the phone sends a fresh link key; both remember each other: the phone in
   its item, the AirPods in theirs) and connect while the lid is open.
4. Put them in your ears (sneak + right-click the case). Settings says **Sound plays on:
   &lt;you&gt;'s AirPods**.

From then on, worn AirPods advertise to their phone and it connects to them by
itself. Put them back in the case and close the lid: they disconnect and the
sound goes back to the phone's speaker. **Disconnect** in Settings keeps them
paired but stops the phone from connecting by itself until **Connect**;
**Forget** unpairs. A phone pairs with any number of devices and connects to one.
Another phone can't connect without the key; pairing a new phone needs the lid
open again.

The console has `bluetoothctl` (`show`, `devices`, `scan [SECONDS]`, `pair DEVICE`,
`connect DEVICE`, `disconnect`, `remove DEVICE`, `power on|off`; DEVICE is an address
or part of a name).

## The headphones layer (audio route)

While the connected AirPods are worn:

- everything the phone plays (`/dev/audio`: Discord, the GBA, `beep`, `aplay`) goes over
  Bluetooth to the AirPods and plays only in the wearer's ears. It is not in the air:
  nobody else hears it and no microphone picks it up (no echo in a call);
- the phone's microphone (`/dev/mic`) is the AirPods': what the sound field gives at the
  wearer's mouth, sent back over Bluetooth at 16 kHz, so the server asks the wearer's
  client for their voice (docs/VOICE.md) and every program recording on the phone hears
  them;
- `/dev/audioctl` reports `headphones 1` (Discord shows **Headphones** and goes full duplex).

The phone sends its microphone wish (whether a program reads `/dev/mic`) in the link's
keep-alive; the AirPods only send their microphone while it is wanted.

## Radio

- Advertising channel 2402 MHz; 37 data channels 2404-2478 MHz (2426 skipped), 2 MHz
  wide, one per link (picked from both addresses). Class 2 power, 4 dBm, chip antennas
  (-1 dBi), 2 Mb/s frames (`BT-EDR2`), sensitivity -90 dBm.
- Frames (`bluetooth/BtFrame.java`): `ADV` (pairable / connectable, battery, the phone it
  is paired with, the name), `PAIR_REQ`/`PAIR_RSP` (the link key), `CONNECT_REQ` (a data
  channel and a proof made from the key)/`CONNECT_RSP`, `AUDIO` and `MIC` (IMA ADPCM
  blocks), `CTRL` (keep-alive: microphone wanted, battery, worn), `DISCONNECT`.
- A link with nothing heard for 2 s is gone (out of range, or behind too much stone);
  worn and paired AirPods advertise again and the phone reconnects when it hears them.

Java is the hardware: `PhoneLink` (the phone's controller) and `HeadsetLink` (the
AirPods' radio) are pure state machines over `BtEndpoint`s on the medium; the phone's
`bluetooth` peripheral (`status`, `scan`, `devices`, `pair`, `connect`, `disconnect`,
`forget`, `set_powered`) is what Settings and `bluetoothctl` (Rust) use.

## Scenario and tests

`airpods` (`/ecm scenario spawn airpods`, GameTest `ecm_bluetooth`): AirPods on and back
into the case; Settings > Bluetooth; control: nothing to pair with the lid closed; lid
open: the AirPods are listed and paired; control: the sound stays on the speaker while
they are not worn; in the ears: Settings says the sound plays on them; `beep` on the
phone plays in the wearer's ears (control: nothing in the air from the phone's speaker);
a Speaker beside the player is recorded loud by the phone's `arecord` through the
AirPods' microphones (their frames reached the phone); `bluetoothctl` shows them; back in
the case with the lid closed, they hang up and the sound is on the speaker again.

Tests: `scripts/Test.ps1 -JUnit BluetoothLinkTest,BluetoothMediumTest -McVersion 26.1`
(frames, pairing, connecting, audio both ways, link loss, the wrong key; and over the
real medium); `cargo test -p ecm-apps --test apps settings` (Settings > Bluetooth);
`scripts/Test.ps1 -GameTests ecm_bluetooth`.
