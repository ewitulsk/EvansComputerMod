# Radio & Wireless — in-game testing guide (1.21.1)

A checklist for verifying PR #52 by hand. For how everything works (every block, program, band and limit), see the [Radio & Wireless guide](RADIO_GUIDE.md). Every section has a **quick check** (a scenario that builds and runs itself) and, where it matters, a **hands-on** build, with what you should see and a **control** that should *not* work.

## 0. Setup

1. Mods folder: `evanscomputermod-mc1.21.1-1.0.0.jar` (repo root), Sable 2.0.5, Create 6.0.10. For section 12 also Create Aeronautics (`create-aeronautics-bundled-1.21.1-1.3.2.jar`).
2. New **creative** superflat world, cheats on. Everything is in the mod's creative tab.
3. Computers only boot once you **open their screen** (right-click the Terminal).
4. Bay modules need a **Module Expansion Card** in the bay first, then the module (click the side of the computer with each).
5. Scenarios: `/ecm scenario list`, `/ecm scenario spawn <name> [auto|manual|fast]` (auto types the script for you and reports each step in chat; manual builds it and prints the commands to type yourself; fast is auto without pauses), `/ecm scenario commands <name>` explains it, `/ecm scenario clear` removes it.

Tip: stand still while an `auto` scenario runs; it prints PASS/FAIL per step in chat.

### How the scenarios are built: exactly what a player does

Every radio scenario is built and run by an invisible helper player (`ScenarioBuilder`, a creative fake player standing next to the build) through the game's own interaction code, so nothing in them is something you couldn't do yourself:

- **Mod blocks** (computers, cables, Access Points, SDRs, speakers, antenna wire, feed points, coax, amplifiers, generators, microwave radios and dishes) are placed by right-clicking with the item, so placement rules, facing, cable/wire connections and ownership are the real ones. A block placed in mid-air is clicked onto a temporary dirt block that is broken again, like you would.
- **Modules** are clicked into the computer's left side (expansion card, then the module).
- **Computers boot** because the helper opens their screen.
- **Access Points** are set up through their screen: the scenario calls the same server handler the screen's *Apply* button sends, with the helper as the player.
- **Tools**: the Antenna Analyzer, RF Wrench and coal are used on the blocks; amplifier/tuner/generator status comes from right-clicking them. The checks read what the game printed in chat.
- **Everything else is typed**: on the computers (`iw`, `wpa_cli`, `wifi`, `dhcpd`, lines in the `python` REPL, `tx_tone`, `radio_station`…) or as chat commands (`/ecm radio link`, `/gamerule`, `/summon lightning_bolt`, `/forceload`, `/give`). In the Python REPL the scenario ends each line with `; print("okN")` so it can see the line ran; you can leave that off.
- **Vanilla terrain** (floors, walls, glass, water, posts, chests with tools in them) is set directly, the same as `/fill` or building it in creative.
- **Files**: big files (the radio station's music) are copied into the computer's storage folder, `<world>/computer-data/<computer id>/`, which is what you do to put files on a computer from outside the game. Small files (`/etc/dhcpd.conf`) are written from the Python REPL with `shell.write_file(...)` (the shell has no `>` redirection or pipes for programs yet; `edit` works too).
- **The one exception** is the airship flight in `airship_radio` (section 12): assembling, flying and landing the ship are scripted.
- **Checks** read what a player would see: the terminal screens, chat/action-bar lines from right-clicks, and the Access Point screen's Status view. A few also read state the screens don't show (the amplifier's FE counter, the pcap file's bytes, the medium's predicted level for the airship) as extra evidence.

---

## 1. Radio basics: walls and wavelength

**Quick check:** `/ecm scenario spawn wifi_walls`
A computer with a Wi-Fi module in the middle of a stone floor, and seven Access Points around it, each ~14 blocks away behind one material: **open air** (east), **glass** (west), **oak planks** (south), **leaves** (north) as 3x3 walls, and **stone** (north-east), **water in a glass tank** (south-east), **iron** (south-west) as 3x3x3 cubes. Each AP is open, channel 11, SSID `walls-<material>`.
- Expect: `iw dev wlan0 scan`, then `wpa_cli scan_results` (one line per network) on the computer: one list has `walls-air`, `walls-glass`, `walls-wood` and `walls-leaves`; no list ever has `walls-stone`, `walls-water` or `walls-iron` (plus two more scans as the control).
- Then the scenario runs `/ecm radio link <computer> <AP> 2462` to every AP and prints a table: stone/water/iron lose 60+ dB more than open air, the others less than 25 dB more.
- Swap a wall (e.g. stone → glass) and scan again: that AP appears. Try `/ecm radio link` between two points with a hill between them at `7` MHz vs `2400` MHz: HF bends round (small diffraction loss), Wi‑Fi doesn't.

## 2. Wi‑Fi access point + phone

**Quick check:** `/ecm scenario spawn wifi_room`
A computer `pc` (10.0.5.1) with a cable under the floor to an Access Point (set up in its screen: SSID `ecm-room`, WPA2, channel 6). Seven blocks further on, two computers with Wi‑Fi modules: `phone` (10.0.5.20, right passphrase) and `rogue` (10.0.5.21, wrong passphrase); both run `wpa_supplicant`.
- Expect: `phone` reaches `wpa_state=COMPLETED`; `pc` pings it 3/3 through the AP; `rogue` never completes, 0/2 pings.
- Right-click the Access Point → **Status** tab: the phone listed with RSSI, rate, handshake DONE; the rogue not authorized.

**Hands-on (AP GUI):**
- Place an Access Point on/next to a network cable (the cable draws an arm to it and the AP's LEDs light).
- Right-click: set SSID, WPA2, passphrase, channel 1/6/11. Close and reopen: passphrase field shows only "set", never the text.
- Control: a second player (or survival non-owner) can't open the settings. Sneak + right-click with the **RF Wrench** → factory reset (open, `ECM-xxxx`).

## 3. Wi‑Fi on a real computer

**Quick check:** `/ecm scenario spawn wifi_wpa2_ping` (a computer with a Wi‑Fi module joins a WPA2 Access Point that is cabled to a gateway computer 192.168.77.1, and pings it; control: no ping before associating), `/ecm scenario spawn wifi_monitor` (monitor mode captures the other computer's probe requests; control: an empty scan with no AP).

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

### 3b. The `wifi` command (one-step join)

**Quick check:** `/ecm scenario spawn wifi_connect`. A "router" computer (192.168.60.1) is cabled to an Access Point (SSID `ecm-cafe`, WPA2 `letmein123`, channel 6) and has `/etc/dhcpd.conf` ready, but `dhcpd` isn't running. A "laptop" computer starts **without** a Wi‑Fi Module.
1. `wifi connect ecm-cafe letmein123` → `wifi: this computer has no Wi-Fi Module` and the fix (Module Expansion Card, then the module, on the side).
2. The card and Wi‑Fi Module are clicked in; `iw dev` shows wlan0.
3. `wifi connect ecm-cafe wrongpass1` → `wifi: the password for 'ecm-cafe' is wrong`.
4. `wifi connect ecm-cafe` → `wifi: 'ecm-cafe' needs a password (WPA2)`.
5. `wifi connect ecm-cafe letmein123` with no DHCP server → `wifi: joined 'ecm-cafe', but nothing gave this computer an address` with the advice to run `dhcpd &`.
6. Router: `dhcpd &`. Laptop: `wifi connect ecm-cafe letmein123` → `Connected. Address 192.168.60.10/24, router 192.168.60.1, ...`; `ping 192.168.60.1` answers; `wifi` shows `wlan0: connected to 'ecm-cafe'`; 4 s later pings still work (the kernel keeps the association after the program exits).

## 4. DHCP (software only)

**Quick check:** `/ecm scenario spawn dhcp_lan` → the server's `/etc/dhcpd.conf` is written from `python` (`import shell`, `shell.write_file("/etc/dhcpd.conf", "pool eth0 ...\n")`), `dhcpd &` runs, the client leases 192.168.50.10; control first: no server → no lease.

**Hands-on:** server: `edit /etc/dhcpd.conf` → `pool eth0 192.168.50.10 192.168.50.100 router 192.168.50.1 dns 1.1.1.1 lease 3600`, `ifconfig eth0 192.168.50.1/24`, `dhcpd &`. Client: `dhclient eth0`, `dhclient -s eth0` (state BOUND).
- Control: a computer cabled **only to the Internet Gateway** running `dhclient eth0` gets **no** lease (the gateway serves no DHCP).

## 5. Wireless Controller on 2.4 GHz

1. Computer with expansion card + **Controller Receiver Module**. Pair the Wireless Controller (right-click the Terminal with it), then right-click in air to connect.
2. Run `controllertest` on the computer. Press bound keys → it reacts. HUD shows `Connected (-NN dBm)`.
3. Walk away / put stone or iron walls between you and the computer → dBm drops, then **"No signal"** (no fixed range any more).
4. Control: remove the receiver module → HUD says **"No receiver: install a Controller Receiver module"**.
5. Alternative receiver: a Wi‑Fi Module switched to controller mode. In `python`: `import peripheral; peripheral.find('wifi').set_mode('controller')`.

## 6. SDR and radio programs

**Quick check:** `/ecm scenario spawn sdr_lab` (B hears A's tone through its Speaker; `scan` finds the carrier; an AFSK1200 packet decodes; off-frequency control decodes nothing; finally `python` → `import radio` works and lists the module's functions). `/ecm scenario spawn radio0_lab` (ping over VHF packet radio).

**Hands-on:** computers A and B, each with an **SDR (Standard)** touching it; a **Speaker** touching B.
- B: `rx_fm 146.52e6` (hiss). A: `tx_tone 146.52e6 --fm 1000 --power 0` → B's speaker plays a tone. Move A away / wall it in → tone gets noisier.
- B: `scan 146e6 147e6` lists A's frequency. `waterfall 146.52e6` draws the spectrum.
- A: `afsk1200 send 144.39e6 N0A N0B hello`; B: `afsk1200 recv 144.39e6` prints it.
- Control: Basic SDR can't transmit (`tx_tone` errors).

## 7. Handheld receiver

The easiest way: spawn the radio station of section 14 and listen to it.

1. A: SDR + `radio_station some.wav 146.52e6 --mode fm --loop` (or `tx_tone 146.52e6 --fm 1000`).
2. Hold a **Handheld Radio**, right-click (on), sneak + right-click → type `146.52` in the frequency box and press Enter (or Scan +). You hear it; the S‑meter shows signal.
3. Walk away → hiss rises and it fades. Squelch up → silence when weak.
4. Control: tune 147.00 MHz → only noise.

## 8. Antennas

**Quick check:** `/ecm scenario spawn ham_dipole` (built wire by wire; the analyzer reads ~7.2 MHz with a 2:1 band; control: the RF Wrench cuts the east arm at the feed point and the analyzer no longer reads a matched 7 MHz dipole, then a second wrench click reconnects it), `/ecm scenario spawn antenna_tools` (the `antenna` program next to the dipole's feed point; control: a lone feed point reads "No antenna").

**Hands-on dipole (7 MHz):** raise a **Feed Point** ~10 blocks up; 10 **Copper Wire** each side along one axis; **Insulator** at each end.
- Right-click the feed point with the **Antenna Analyzer**: "Resonant at ~7.x MHz · 2:1 SWR band … · rated … W (copper wire) …". Sneak + right-click: SWR plot screen.
- Computer touching the feed point: `antenna`, `antenna swr 6e6 8e6 21` (minimum near 7.2 MHz), `antenna z 7.1e6`, `antenna polar 7.1e6 az`, `antenna limits --amp 1k`.
- Make one arm longer → resonance drops. Touch an iron block to the wire → it detunes. Replace a middle wire with an insulator → the antenna splits.
- RF Wrench on a wire side cuts that connection (the arm disappears).
- Copper oxidizes over time; honeycomb waxes it.
- Control: bare feed point → "No antenna".

## 9. Power, amplifiers, hazards

**Quick check:** `/ecm scenario spawn ham_station` (Burner Generator → 100 W amplifier → tuner → arrestor → feed point → HF dipole, SDR + computer). The script: the analyzer reads the dipole; **control 1** before any fuel: `tx_tone 7.1M --seconds 1 --power 37` and a right-click on the amplifier shows BROWNOUT, a 5 W bypass, nothing drawn; then coal goes into the generator (right-click), the amplifier charges, `tx_tone 7.1M --seconds 2 --power 37` gives ~100 W out (amplifier right-click) and ~1600 FE drawn; **control 2**: `/gamerule radioHazards 1`, `/gamerule radioLightningDamage true`, `/summon lightning_bolt` on the dipole → the arrestor grounds it, nothing breaks (gamerules put back afterwards).

**Hands-on:**
- **Burner Generator:** add coal → lit, flame; right-click empty-handed for FE status.
- **Chain:** SDR touching an **Amplifier (100 W)**, coax from the amp to a **Tuner**, coax through a **Lightning Arrestor** to the feed point. Power the amp from the generator. Transmit from the SDR (`tx_tone 7.1e6 --power 30`): a far receiver hears it much louder than without the amp; the amp only draws FE while transmitting (right-click it for status). Remove fuel → brownout (weaker, no damage).
- **Hazards** (default gamerule `radioHazards` = server default = *equipment*):
  - 1 kW amp into a **thin copper-wire** dipole → wire glows/sizzles, then the hottest segment melts and drops **Melted Scrap**. Same with **Heavy Cable** → survives (control).
  - `/gamerule radioHazards 0` → warnings only, nothing breaks.
  - Lightning: summon `/summon lightning_bolt` at a tall antenna **without** an arrestor → the amp/SDR down the coax is destroyed; **with** an arrestor → nothing (control). `/gamerule radioLightningDamage false` disables it.
  - **RF Meter** held near a transmitting element shows V/m and the exposure limit (damage only at `radioHazards 2`).

## 10. Microwave links

**Quick check:** `/ecm scenario spawn microwave_link`. Each host has a Microwave Radio on top (cabled round to its eth0) and a 1.2 m dish beside it. In the `python` REPL on each host: `import peripheral`, `r = peripheral.find("microwave_radio")`, `d = peripheral.find("dish")`, `r.set_tx_power(-40)`, `d.aim_at(x, y, z)` with the far dish's centre. Pings cross 3/3; control: `d.nudge(30, 0)` on the east dish → 0/2; `d.align(40)` → `{'found': True, ...}` and 2/2 again.

**Hands-on:** two cable networks far apart, each with a **Microwave Radio** on the cable and a **Dish** next to it, aimed at each other (sneak + right-click the dish edges to nudge yaw/pitch; the action bar shows aim and link). Ping across.
- Control: turn one dish away → link lost; `align()` from a computer touching the dish (`peripheral.find('dish').align()`) restores it.
- `/weather thunder` on a 60 GHz long hop → the link fades or drops.

## 11. Ships (Sable)

Assemble a small structure carrying an AP + computer (or a dipole + feed point), then fly/move it:
- Link and settings survive assembly and disassembly; Wi‑Fi re-associates afterwards.
- Rotating a ship carrying a dipole changes what a ground receiver hears (pattern + polarization).
- A ship hull between two ground radios weakens their link (`/ecm radio link` through it).

## 12. Airships (Create Aeronautics)

**Quick check (needs Aeronautics):** `/ecm scenario spawn airship_radio`. The computer on the ship (cabled to an AP) pings a **ground computer** with a Wi‑Fi module (`phone`, 10.0.7.20, `wpa_supplicant`) in a wooden shack; the strip the ship flies over is kept loaded with `/forceload`. `iw dev wlan0 link` on the phone shows the signal and rate falling with distance; at 500 m `wpa_cli status` leaves COMPLETED and pings die; back at 14 m it re-associates by itself. AP config survives landing.
**Not player-equivalent:** the ship's assembly, flight and landing are scripted (Sable assembly and held positions). A player would use Create Simulated's Physics Assembler (hold its lever) and fly with propellers and controls; that can't be driven reliably from a scenario. Everything radio in it is real.

## 13. Modpack bits (optional)

- `config/…/serverconfig/evanscomputermod-server.toml`: set `power.burnerGenerator.enabled = false` → recipe gone, not in creative tab, existing generators say "disabled by server config".
- KubeJS (if installed): cancel `RadioTransmitEvent` via NativeEvents → SDR transmit fails with an I/O error.

## 14. Radio station: music on shortwave (try the Handheld Radio)

**Quick check:** `/ecm scenario spawn radio_station` (or `manual` to start the station yourself).

What gets built (all by right-clicks):
- **Station:** a computer with a **Standard SDR** on its east side; **coax** along the ground from the SDR to the antenna.
- **Custom antenna, a ground-mounted quarter-wave vertical for the 25 m broadcast band**, 10 blocks east of the computer: a **Feed Point** clicked onto the grass (vertical axis: it feeds the wire against the ground), **6 blocks of Copper Wire** straight up, an **Insulator** on top. The analyzer reads about "Resonant at 11.6 MHz · 2:1 SWR band 11.5–11.9 MHz".
  It stands on the ground on purpose: the antenna solver's patterns are zero below an antenna's horizon, so a raised antenna is barely heard by receivers at ground level nearby.
- **Listener:** 20 blocks south, a computer with a **Basic SDR** and a **Speaker**.
- **Chest** (west of the station): a **Handheld Radio** and an **Antenna Analyzer**.
- **Music:** five public-domain / CC0 recordings (Mendelssohn's Wedding March, Bach's *Jesu, Joy of Man's Desiring* and the Goldberg Aria, a Scriabin prelude, Vivaldi's Mandolin Concerto RV 425; 8 kHz mono WAV, up to 3 min each, credits in `/radio/CREDITS.txt`), copied into the station computer's storage folder `<world>/computer-data/<id>/radio/` with a `playlist.m3u`.

The script:
1. Antenna Analyzer on the feed point: resonant in the 25 m band, **11.600 MHz** inside its 2:1 SWR band.
2. Station: `ls /radio`, then `radio_station /radio/playlist.m3u 11.6M --mode am --loop --power 0 &` → `now playing: Mendelssohn - Wedding March …`, and a new `now playing:` line for every song, looping forever. 0 dBm (1 mW) is plenty here; at the SDR's full 37 dBm the listener 20 blocks away overloads.
3. `/ecm radio link` from the feed point to the listener prints the path.
4. Listener: `rx_am 11.6M --seconds 6` plays 6 s on its Speaker.
5. A Handheld Radio's receiver held at the listener (the same server-side code the item runs for you): SW 11.600 MHz shows the station (about -50 dBm, music) while 11.670 MHz is only noise (about -120 dBm) — the control.
6. `/give` hands everyone within 64 blocks a Handheld Radio; the station keeps playing (`jobs` shows it).

**Listen with the Handheld Radio:** hold it (either hand), right-click to switch it on, sneak + right-click to open tuning. Type **11.6** in the box at the top and press Enter (it picks the SW band for you), or press **Scan +** / **Scan -**, which jump straight to the next station you can hear on the band. The S-meter fills (tens of dBm near the station); music plays through the **Jukebox/Note Blocks** volume slider, so check that isn't at 0. Tune off a few steps (`>`): only noise. Squelch up → silence between songs is cut.

The frequency box takes `11.6` (MHz), `1000` or `1000k` (kHz, AM band), `146.52` (VHF) or explicit units (`11.6M`, `1000 kHz`).

**Hands-on:** put your own WAV files (PCM 8/16-bit, any rate; mono is smallest) in `<world>/computer-data/<id>/radio/` (or any folder), then `radio_station /radio 11.6M --mode am --loop` plays every WAV in the folder in name order, or list files / a playlist (one path per line) before the frequency. `--gap S` sets the silence between songs. FM on VHF works too (`--mode fm 146.52M`, Handheld VHF band).
To stop it: `ps` (or `jobs`) and `kill <pid>`, or break the station's computer. `/ecm scenario clear` removes everything.

## 15. Fixed while making these scenarios player-built

Building everything the way a player does turned up real bugs, now fixed:
- **Access Point ↔ computer Wi‑Fi:** the AP block never sent 802.11 ACKs and kept the module's FCS on received frames, so a computer's Wi‑Fi Module (which retries unacknowledged frames, then gives up) could never authenticate with a real Access Point. The old scenarios only used virtual phones/APs, which hid it.
- **Handheld Radio heard no AM audio near a station:** its receiver had a fixed +40 dB gain (full scale at -50 dBm), so a nearby station clipped. Clipping flattens an AM envelope (the audio) while the carrier still quiets the static. It now has AGC with a front-end attenuator (+40 to -60 dB); `handheld_hears_am_station` checks a 1 kHz AM tone at 3, 20 and 150 blocks.
- **Wi‑Fi scans at normal speed:** a computer scanned each channel for 30 ms, but an Access Point answers probes on its server tick (50 ms) and beacons every 100 ms, so scans often missed a network right next to it. The active dwell is now 120 ms (a full scan takes ~1.6 s).
- **Python REPL:** what you typed wasn't echoed (you typed blind), and `shell.write_file` returned False on success and didn't accept absolute paths; `python /file.py` couldn't open absolute paths either.
- **AP scenarios in GameTests** now run the server at 20 ticks per second: the AP's EAPOL timers run on game time, and an unthrottled test server timed the handshake out before a real computer could answer.

## What to report back

For anything that misbehaves: the section number, what you did, what you saw, and `logs/latest.log`. Known gaps (not bugs) are listed in PR #52's description.
