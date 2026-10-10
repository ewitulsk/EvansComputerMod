# Radio & Wireless — in-game testing guide (1.21.1)

A checklist for verifying PR #52 by hand. Every section has a **quick check** (a scenario that builds and runs itself) and, where it matters, a **hands-on** build, with what you should see and a **control** that should *not* work.

## 0. Setup

1. Mods folder: `evanscomputermod-mc1.21.1-1.0.0.jar` (repo root), Sable 2.0.5, Create 6.0.10. For section 12 also Create Aeronautics (`create-aeronautics-bundled-1.21.1-1.3.2.jar`).
2. New **creative** superflat world, cheats on. Everything is in the mod's creative tab.
3. Computers only boot once you **open their screen** (right-click the Terminal).
4. Bay modules need a **Module Expansion Card** in the bay first, then the module (click the side of the computer with each).
5. Scenarios: `/ecm scenario list`, `/ecm scenario spawn <name> [auto|manual|fast]` (auto types the script for you and reports each step in chat; manual builds it and prints the commands to type yourself), `/ecm scenario commands <name>` explains it, `/ecm scenario clear` removes it.

Tip: stand still while an `auto` scenario runs; it prints PASS/FAIL per step in chat.

---

## 1. Radio basics: walls and wavelength

**Quick check:** `/ecm scenario spawn wifi_walls`
Seven lanes, each with a transmitter and receiver 14 blocks apart through: air, glass, wood, leaves, stone, water, iron.
- Expect: chat table: air/glass/wood/leaves *delivered*; stone/water/iron *lost*; stone ≈ 60+ dB below open air.
- Swap a wall block (e.g. stone → glass), then run `/ecm radio link <tx x y z> <rx x y z> 2472` on that lane. The "walls" term changes.
- Try `/ecm radio link` between two points with a hill between them at `7` MHz vs `2400` MHz: HF bends round (small diffraction loss), Wi‑Fi doesn't.

## 2. Wi‑Fi access point + phone

**Quick check:** `/ecm scenario spawn wifi_room`
- Expect: the computer pings the virtual phone (lime carpet) 3/3 through the Access Point; the rogue (red carpet, wrong passphrase) gets 0/2.
- Right-click the Access Point → **Status** tab: phone listed with RSSI, rate, handshake DONE; rogue shows MIC failures.

**Hands-on (AP GUI):**
- Place an Access Point on/next to a network cable (the cable draws an arm to it and the AP's LEDs light).
- Right-click: set SSID, WPA2, passphrase, channel 1/6/11. Close and reopen: passphrase field shows only "set", never the text.
- Control: a second player (or survival non-owner) can't open the settings. Sneak + right-click with the **RF Wrench** → factory reset (open, `ECM-xxxx`).

## 3. Wi‑Fi on a real computer

**Quick check:** `/ecm scenario spawn wifi_wpa2_ping` (WPA2 association + ping), `/ecm scenario spawn wifi_monitor` (monitor mode capture).

**Hands-on:** AP on a cable to computer A (`ifconfig eth0 192.168.1.1/24`). Computer B: expansion card + **Wi‑Fi Module**. On B:
```
iw dev                      # wlan0 exists
iw dev wlan0 scan           # your SSID, channel, RSSI
edit /etc/wpa_supplicant.conf
    network={
        ssid="my-ssid"
        psk="my-passphrase"
    }
wpa_supplicant -B -i wlan0 -c /etc/wpa_supplicant.conf
wpa_cli status              # wpa_state=COMPLETED
ifconfig wlan0 192.168.1.50/24
ping 192.168.1.1
iw dev wlan0 link           # signal (dBm) and tx bitrate
```
- Move the **AP** further away or put walls between it and B: `iw dev wlan0 link` shows a weaker signal and lower bitrate; enough stone and the link drops.
- Control: wrong `psk` → `wpa_cli status` never reaches COMPLETED, ping fails.
- Monitor: `iw dev wlan0 set type monitor`, `tcpdump -i wlan0 -c 10 -w cap.pcap` shows beacons/probes (pcap opens in Wireshark).

## 4. DHCP (software only)

**Quick check:** `/ecm scenario spawn dhcp_lan` → client leases 192.168.50.10; control first: no server → no lease.

**Hands-on:** server: `edit /etc/dhcpd.conf` → `pool eth0 192.168.50.10 192.168.50.100 router 192.168.50.1 dns 1.1.1.1 lease 3600`, `ifconfig eth0 192.168.50.1/24`, `dhcpd &`. Client: `dhclient eth0`, `dhclient -s eth0` (state BOUND).
- Control: a computer cabled **only to the Internet Gateway** running `dhclient eth0` gets **no** lease (the gateway serves no DHCP).

## 5. Wireless Controller on 2.4 GHz

1. Computer with expansion card + **Controller Receiver Module**. Pair the Wireless Controller (right-click the Terminal with it), then right-click in air to connect.
2. Run `controllertest` on the computer. Press bound keys → it reacts. HUD shows `Connected (-NN dBm)`.
3. Walk away / put stone or iron walls between you and the computer → dBm drops, then **"No signal"** (no fixed range any more).
4. Control: remove the receiver module → HUD says **"No receiver: install a Controller Receiver module"**.
5. Alternative receiver: a Wi‑Fi Module switched to controller mode. In `python`: `import peripheral; peripheral.find('wifi').set_mode('controller')`.

## 6. SDR and radio programs

**Quick check:** `/ecm scenario spawn sdr_lab` (B hears A's tone through its Speaker; `scan` finds the carrier; an AFSK1200 packet decodes; off-frequency control decodes nothing). `/ecm scenario spawn radio0_lab` (ping over VHF packet radio).

**Hands-on:** computers A and B, each with an **SDR (Standard)** touching it; a **Speaker** touching B.
- B: `rx_fm 146.52e6` (hiss). A: `tx_tone 146.52e6 --fm 1000 --power 0` → B's speaker plays a tone. Move A away / wall it in → tone gets noisier.
- B: `scan 146e6 147e6` lists A's frequency. `waterfall 146.52e6` draws the spectrum.
- A: `afsk1200 send 144.39e6 N0A N0B hello`; B: `afsk1200 recv 144.39e6` prints it.
- Control: Basic SDR can't transmit (`tx_tone` errors).

## 7. Handheld receiver

1. A: SDR + `radio_station some.wav 146.52e6 --mode fm --loop` (or `tx_tone 146.52e6 --fm 1000`).
2. Hold a **Handheld Radio**, right-click (on), sneak + right-click → tune VHF to 146.52 MHz. You hear it; the S‑meter HUD shows signal.
3. Walk away → hiss rises and it fades. Squelch up → silence when weak.
4. Control: tune 147.00 MHz → only noise.

## 8. Antennas

**Quick check:** `/ecm scenario spawn ham_dipole`, `/ecm scenario spawn antenna_tools`.

**Hands-on dipole (7 MHz):** raise a **Feed Point** ~10 blocks up; 10 **Copper Wire** each side along one axis; **Insulator** at each end.
- Right-click the feed point with the **Antenna Analyzer**: "Resonant at ~7.x MHz · 2:1 SWR band … · rated … W (copper wire) …". Sneak + right-click: SWR plot screen.
- Computer touching the feed point: `antenna`, `antenna swr 6e6 8e6 21` (minimum near 7.2 MHz), `antenna z 7.1e6`, `antenna polar 7.1e6 az`, `antenna limits --amp 1k`.
- Make one arm longer → resonance drops. Touch an iron block to the wire → it detunes. Replace a middle wire with an insulator → the antenna splits.
- RF Wrench on a wire side cuts that connection (the arm disappears).
- Copper oxidizes over time; honeycomb waxes it.
- Control: bare feed point → "No antenna".

## 9. Power, amplifiers, hazards

**Quick check:** `/ecm scenario spawn ham_station` (Burner Generator → amplifier → tuner → arrestor → feed point → HF dipole, SDR + computer).

**Hands-on:**
- **Burner Generator:** add coal → lit, flame; right-click empty-handed for FE status.
- **Chain:** SDR touching an **Amplifier (100 W)**, coax from the amp to a **Tuner**, coax through a **Lightning Arrestor** to the feed point. Power the amp from the generator. Transmit from the SDR (`tx_tone 7.1e6 --power 30`): a far receiver hears it much louder than without the amp; the amp only draws FE while transmitting (right-click it for status). Remove fuel → brownout (weaker, no damage).
- **Hazards** (default gamerule `radioHazards` = server default = *equipment*):
  - 1 kW amp into a **thin copper-wire** dipole → wire glows/sizzles, then the hottest segment melts and drops **Melted Scrap**. Same with **Heavy Cable** → survives (control).
  - `/gamerule radioHazards 0` → warnings only, nothing breaks.
  - Lightning: summon `/summon lightning_bolt` at a tall antenna **without** an arrestor → the amp/SDR down the coax is destroyed; **with** an arrestor → nothing (control). `/gamerule radioLightningDamage false` disables it.
  - **RF Meter** held near a transmitting element shows V/m and the exposure limit (damage only at `radioHazards 2`).

## 10. Microwave links

**Quick check:** `/ecm scenario spawn microwave_link` (pings 3/3 across the link, 0/2 with a dish turned 30°, 2/2 after `align()`).

**Hands-on:** two cable networks far apart, each with a **Microwave Radio** on the cable and a **Dish** next to it, aimed at each other (sneak + right-click the dish edges to nudge yaw/pitch; the action bar shows aim and link). Ping across.
- Control: turn one dish away → link lost; `align()` from a computer (`peripheral.find('dish').align()`) restores it.
- `/weather thunder` on a 60 GHz long hop → the link fades or drops.

## 11. Ships (Sable)

Assemble a small structure carrying an AP + computer (or a dipole + feed point), then fly/move it:
- Link and settings survive assembly and disassembly; Wi‑Fi re-associates afterwards.
- Rotating a ship carrying a dipole changes what a ground receiver hears (pattern + polarization).
- A ship hull between two ground radios weakens their link (`/ecm radio link` through it).

## 12. Airships (Create Aeronautics)

**Quick check (needs Aeronautics):** `/ecm scenario spawn airship_radio`. The computer on the ship pings a ground phone through the AP; RSSI and rate fall with distance, the link drops far away and re-associates on return. AP config survives landing.

## 13. Modpack bits (optional)

- `config/…/serverconfig/evanscomputermod-server.toml`: set `power.burnerGenerator.enabled = false` → recipe gone, not in creative tab, existing generators say "disabled by server config".
- KubeJS (if installed): cancel `RadioTransmitEvent` via NativeEvents → SDR transmit fails with an I/O error.

## What to report back

For anything that misbehaves: the section number, what you did, what you saw, and `logs/latest.log`. Known gaps (not bugs) are listed in PR #52's description.
