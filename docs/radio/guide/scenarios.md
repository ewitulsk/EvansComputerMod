# Radio scenarios

Part of the [Radio & Wireless guide](../RADIO_GUIDE.md). For a hands-on test checklist see
[IN_GAME_TESTING.md](../IN_GAME_TESTING.md).

A scenario builds a working setup in front of you and (optionally) types the commands for you,
checking each step. They are the fastest way to see a feature work, and every one is built the
way a player would build it.

## Commands (operator)

| Command | |
|---|---|
| `/ecm scenario list` | every scenario with a one-line description |
| `/ecm scenario spawn <name> [auto\|manual\|fast]` | build it. `auto` (default) types the script, pausing 4 ticks per step, and reports each step in chat (`ok ...` / `FAIL ...`); `manual` builds and boots, prints the walkthrough and says `<name>: terminals are up, go ahead.`; `fast` = auto without pauses |
| `/ecm scenario commands <name>` | print the walkthrough: `[node]` and the lines to type, `=> what to expect`, `-- wait ...` |
| `/ecm scenario rerun`, `status`, `clear` | rerun the last, show progress, remove everything (`Removed n scenario(s).`) |

The layout is centred on you in x and starts 3 blocks south (+z) at your feet level; its area
(plus a 1-block margin) is cleared to air first. Stand still while `auto` runs. Coordinates
below are relative to that origin (+x east, +z south). Failures dump the terminal screens to
the server log; every message is also logged as `[scenario <name>] ...`.

**How they are built.** An invisible creative helper player ("ScenarioBuilder") places every mod
block by right-clicking with the item, clicks modules into computers (expansion card first, left
side), boots computers by opening their screens, sets Access Points up through the same handler
as the GUI's Apply button, and uses tools (analyzer, wrench, coal) on blocks. Vanilla terrain
(floors, posts, walls, chests) is set directly like `/fill`. Large files are copied into the
computer's storage folder; small ones are written from Python with `shell.write_file`. The only
non-player step anywhere is the airship's assembly and flight in `airship_radio`.

## The 14 radio scenarios

| Scenario | Shows | Script time |
|---|---|---|
| [`wifi_walls`](#wifi_walls) | what blocks do to 2.4 GHz | 4.8 s |
| [`wifi_room`](#wifi_room) | AP + WPA2 + a phone; a rogue with the wrong passphrase | 15.9 s |
| [`wifi_wpa2_ping`](#wifi_wpa2_ping) | joining by hand with `wpa_cli` + `wpa_supplicant` | 9.8 s |
| [`wifi_connect`](#wifi_connect) | the `wifi` command and every error it explains | 39.2 s |
| [`wifi_monitor`](#wifi_monitor) | monitor mode, radiotap pcap | 3.2 s |
| [`dhcp_lan`](#dhcp_lan) | `dhcpd` / `dhclient` on a cable | 7.0 s |
| [`sdr_lab`](#sdr_lab) | FM tone, scan, AFSK1200 packet, `import radio` | 26.9 s |
| [`radio0_lab`](#radio0_lab) | ping over VHF packet radio | 13.3 s |
| [`radio_station`](#radio_station) | music on shortwave, handheld listening | 14.9 s |
| [`ham_dipole`](#ham_dipole) | building a dipole; analyzer; wrench cut | 0.2 s |
| [`antenna_tools`](#antenna_tools) | the `antenna` program | 0.3 s |
| [`ham_station`](#ham_station) | generator → amplifier → tuner → arrestor → dipole; lightning | 6.1 s |
| [`microwave_link`](#microwave_link) | dishes, aiming, `align` | 22.2 s |
| [`airship_radio`](#airship_radio) | Wi-Fi on a flying airship (needs Sable) | 48.0 s |

(Script times in the 2026-10-10 GameTest run `merged-v3`, after building and booting; `auto` mode in a real game is slower because it paces each step.)

### wifi_walls

- **Built**: a computer with a Wi-Fi Module at the centre of a 31×31 stone floor; seven **open**
  Access Points on channel 11 (2462 MHz), 20 dBm, SSID `walls-<material>`, each ~14 blocks out
  behind one material: air (east), glass (west), oak planks (south), leaves (north) as 1×3×3
  walls; stone (north-east), water in a glass tank (south-east), iron (south-west) as 3×3×3 cubes.
- **Script**: `iw dev wlan0 scan` + `wpa_cli scan_results` until `walls-air`, `walls-glass`,
  `walls-wood`, `walls-leaves` are listed; `walls-stone`, `walls-water`, `walls-iron` must never
  appear (two extra scans as the control). Then `/ecm radio link` from the computer to each AP
  at 2462 MHz, printed as a table ([real output](propagation.md#21-walls-near-the-two-ends)).
- **Pass**: glass/wood/leaves lose ≤ 25 dB more than air; stone/water/iron ≥ 60 dB more.

### wifi_room

- **Built**: `pc` (10.0.5.1, eth0) with a cable under the floor to an Access Point 5 blocks south
  (SSID `ecm-room`, WPA2 `correct horse battery`, channel 6, 20 dBm); 7 blocks further, `phone`
  and `rogue` with Wi-Fi Modules.
- **Script**: phone joins with the right passphrase (the `wpa_cli` sequence below), rogue with
  `not the passphrase`; phone reaches `wpa_state=COMPLETED`; pc pings the phone 3/3; rogue never
  completes and pc's 2 pings to it fail; the AP's **Status** tab lists the phone
  `AUTHORIZED/DONE` (real: `phone AUTHORIZED/DONE at -35 dBm; rogue not listed`).

### wifi_wpa2_ping

- **Built**: `pc` with a Wi-Fi Module; `gw` (192.168.77.1) cabled to an AP (SSID `ecm-lab`, WPA2
  `correct horse battery`, channel 6).
- **Script**: `ifconfig wlan0 192.168.77.2/24`; control ping fails before joining;
  `wpa_cli add_network` → `0`, `set_network 0 ssid ecm-lab`, `set_network 0 psk "correct horse battery"`,
  `enable_network 0`; scan until listed; `wpa_supplicant -B -D packet -i wlan0 -c /etc/wpa_supplicant.conf`;
  `wpa_cli status` until `COMPLETED`; ping 3/3; `iw dev wlan0 link` shows `tx bitrate`;
  `wpa_cli list_networks` shows `0  ecm-lab  any  [CURRENT]`.

### wifi_connect

- **Built**: `router` (192.168.60.1) cabled to an AP (SSID `ecm-cafe`, WPA2 `letmein123`,
  channel 6), with `/etc/dhcpd.conf` = `pool eth0 192.168.60.10 192.168.60.100 router 192.168.60.1 dns 1.1.1.1 lease 3600`
  but `dhcpd` not running; a `laptop` **without** a Wi-Fi Module.
- **Script**: `wifi connect ecm-cafe letmein123` → no module (with the fix); the card and module
  are clicked in; `wifi connect ecm-cafe wrongpass1` → wrong password; `wifi connect ecm-cafe` →
  needs a password; the right password → joined but no address (advice names `dhcpd &`); router
  `dhcpd &`; connect again → `Connected. Address 192.168.60.N/24, router 192.168.60.1`; ping;
  `wifi` → `wlan0: connected to 'ecm-cafe'`; 4 s later pings still work.

### wifi_monitor

- **Built**: computers `a` and `b` with Wi-Fi Modules, no AP.
- **Script**: `iw dev` on both; control: `iw dev wlan0 scan` on `a` lists nothing; `a`:
  `iw dev wlan0 set type monitor`, `iw dev wlan0 set channel 1`, `iw dev wlan0 info` (type
  monitor), `tcpdump -i wlan0 -c 3 -w probes.pcap` while `b` scans → `3 packets captured`; the
  pcap is checked (link type 127, three radiotap probe requests from b's MAC); then
  `tcpdump -i wlan0 -c 2` prints `Probe Request () SA:<b's MAC> DA:ff:ff:ff:ff:ff:ff`.

### dhcp_lan

- **Built**: `server` and `client` joined by a cable (no Wi-Fi: DHCP is software).
- **Script**: control first: client `dhclient -t 4 eth0` → `no lease on eth0 after 4s`,
  `dhclient -x eth0`; server `ifconfig eth0 192.168.50.1/24`, writes
  `pool eth0 192.168.50.10 192.168.50.100 router 192.168.50.1 dns 1.1.1.1 lease 3600`,
  `dhcpd &` → `serving eth0 192.168.50.10-192.168.50.100/24 as 192.168.50.1`; client
  `dhclient eth0` → `bound to 192.168.50.10/24`, ping, `dhclient -s eth0` → `state BOUND`,
  `dhclient -r eth0` → `released 192.168.50.10`.

### sdr_lab

- **Built**: computers A and B 12 blocks apart, each with a Standard SDR on its east side; a
  Speaker on B's west side.
- **Script**: B `rx_fm 146.52M --seconds 8` while A `tx_tone 146.52M --fm 1000 --seconds 5 --power 0`
  → B `fm: strongest audio tone 996 Hz, 90 dB over the noise`; A `tx_tone 146.52M --offset 5k --seconds 6 --power 0`
  while B `scan 146.45M 146.6M --dwell 300` finds 146.525 MHz; B `afsk1200 recv 144.39M --count 1 --seconds 15`
  while A `afsk1200 send 144.39M N0CALL-1 APRS hello from A --power 0` → `N0CALL-1>APRS:hello from A`;
  control: B listens on 145.00 MHz → `afsk1200: 0 frames decoded`; A `python`, `import radio`,
  lists the module's names.

### radio0_lab

- **Built**: the `sdr_lab` bench without the speaker.
- **Script**: A `radiod radio0 up sdr_0 144.39M --call N0CALL-1 --ip 10.44.0.1/24 --seconds 50 --gain 10 --txdelay 100 -v --power 0 &`,
  B the same with `N0CALL-2` / `10.44.0.2/24`; A `ping 10.44.0.2 -n 3` (1–3 replies);
  control `ping 10.44.0.9 -n 1` → 0 received.

### radio_station

- **Built**: a station computer with a Standard SDR; Coax along the ground 10 blocks east to a
  **ground-mounted quarter-wave vertical**: Feed Point on the grass, 6 Copper Wire straight up,
  an Insulator on top. 20 blocks south, a listener computer with a Basic SDR and a Speaker. A
  chest with a Handheld Radio and an Antenna Analyzer. Five public-domain/CC0 recordings
  (Mendelssohn's Wedding March, Bach's *Jesu, Joy of Man's Desiring* and Goldberg Aria, a Scriabin
  prelude, Vivaldi's Mandolin Concerto RV 425; 8 kHz mono WAV, ≤ 3 min, credits in
  `/radio/CREDITS.txt`) and `playlist.m3u` in the station's `/radio/`.
- **Script**: analyzer on the feed point → `Resonant at 11.6 MHz · 2:1 SWR band 11.5–11.9 MHz · rated 22 W (copper wire) / 789 W (insulators)`;
  `ls /radio`; `radio_station /radio/playlist.m3u 11.6M --mode am --loop --power 0 &` →
  `now playing: Mendelssohn - Wedding March ...`; `/ecm radio link` feed → listener
  (`21.9 m @ 11.600 MHz: total 59.1 dB (GROUND_WAVE, NLOS) ...`); listener `rx_am 11.6M --seconds 6`
  plays on the Speaker; a handheld receiver at the listener: SW 11.600 MHz −34 dBm vs 11.670 MHz
  −120 dBm (the control); `/give @a[distance=..64] evanscomputermod:handheld_radio`; `jobs` shows
  the station still running (forever, until its computer is broken).
- **Listen**: hold the Handheld Radio, right-click (on), sneak + right-click, type `11.6`, Enter
  (or Scan +).

### ham_dipole

- **Built**: two spruce fence posts 22 blocks apart, Insulators on top (y 10), Copper Wire
  inward from each insulator (2 × 10 blocks), a Feed Point clicked into the gap last (axis X);
  a chest with an Antenna Analyzer and an RF Wrench.
- **Script**: analyzer on the feed point (re-clicked while `solving`) → resonance between 0.88 and
  1.0 × 7.49 MHz with a 2:1 band (real: `Resonant at 7.3 MHz · 2:1 SWR band 7.1–7.4 MHz · rated 61 W (copper wire) / 2.1 kW (insulators)`);
  control: the wrench cuts the feed point's east side (`Cut the east connection`) → `No resonance found · best SWR 39.4:1 at 13.6 MHz`;
  the wrench restores it.

### antenna_tools

- **Built**: the `ham_dipole` antenna plus computer A against the feed point's north (coax) side;
  computer B next to a lone Feed Point on the grass.
- **Script** on A: `antenna` (6.6–7.4 MHz), `antenna swr 6e6 8e6 21`, `antenna z 7.1M`,
  `antenna limits --amp 1k` (`... amp 1.0 kW -> will overheat`); B: `antenna` → `No antenna`.
  Real screens in [Antennas](antennas.md#the-antenna-program).

### ham_station

- **Built** (all by right-clicks): a 2 × 10 **Antenna Wire** dipole 10 blocks up on posts with
  insulators; Feed Point; Coax down the mast; a **Lightning Arrestor**; an **Antenna Tuner**; Coax;
  a **Power Amplifier (100 W)** with an unfuelled **Burner Generator** beside it; a Standard SDR
  and the computer `radio`. A chest with an RF Meter, an Antenna Analyzer and 32 coal.
- **Script**: `peripherals` shows the SDR; analyzer → `Resonant at 7.2 MHz ... rated 228 W (antenna wire) ... lightning arrestor fitted`;
  control 1 (no fuel): `tx_tone 7.1M --seconds 1 --power 37`, amplifier right-click →
  `out 5.0 W (drive 5.0 W) ... 0 / 16000 FE ... BROWNOUT 0% supply`; coal into the generator;
  once the amplifier holds ≥ 2000 FE, `tx_tone 7.1M --seconds 2 --power 37` → `out 100.0 W`,
  SWR 1.0:1 at the amplifier thanks to the tuner (`antenna SWR 1.8:1 matched to 1.0:1, absorbing 6 W`),
  1560 FE drawn, antenna 85.7 W; control 2: `/gamerule radioHazards 1`,
  `/gamerule radioLightningDamage true`, `/summon minecraft:lightning_bolt` on the east arm →
  nothing destroyed, `lightning grounded by the arrestor`; the gamerules are put back.

### microwave_link

- **Built**: two hosts 28 blocks apart (10.60.0.1 and .2), each with a Microwave Radio on top
  (cable from the host's bottom round to it) and a 1.2 m Dish beside it facing the other
  (faces 26 blocks apart); 24 GHz, channel 0, 56 MHz.
- **Script** (Python on each host): `import peripheral`, `r = peripheral.find("microwave_radio")`,
  `d = peripheral.find("dish")`, `r.set_tx_power(-40)`, `d.aim_at(<far dish centre>)`; pings 3/3;
  control: `d.nudge(30, 0)` on the east dish → 0/2; `d.align(40)` → `{'found': True, ...}`;
  pings 2/2.

### airship_radio

Needs **Sable**; uses Create Aeronautics envelopes and Create Simulated's assembler when
installed, otherwise white wool and iron bars and Sable's own assembler.

- **Built**: a deck with `pc` (10.0.7.1) cabled to an AP (SSID `ecm-airship`, WPA2
  `up up and away`, channel 1, **0 dBm**) under an envelope balloon; on the ground 8 blocks west,
  `phone` (10.0.7.20, Wi-Fi Module) in a wooden shack. `/forceload` keeps the 500-block flight
  strip loaded.
- **Script**: phone joins, pc pings it; **[scripted]** assemble the ship; pings from the ship;
  fly to 14 m: 4 × `iw dev wlan0 link` → `signal -75.0 dBm` (medium predicts −71.1),
  rate 36 Mb/s; 40 m: −85 dBm (predicts −81.0), 9 Mb/s; control: 500 m (predicts −136.5 dBm) →
  the phone leaves `COMPLETED` and pings fail; back at 14 m it re-associates by itself; land;
  AP settings, passphrase and cable unchanged; pings.
- The assembly, flight and landing are scripted (a player would use the Physics Assembler and
  propellers); everything radio is real.

## GameTests

All of the above run as GameTests (namespace `ecm_radio`, 60 tests), together with focused tests
of the medium, antennas, power, hazards, Wi-Fi, handheld, controller, microwave and ships. Run:

```
pwsh scripts/Test.ps1 -Area radio -GameTests ecm_radio -McVersion 1.21.1 -Aeronautics
```

(`-Aeronautics` needs `libs/optional/create-aeronautics-bundled-1.21.1-*.jar`; without it the
Aeronautics-specific checks are skipped and logged.) Receipts land in `artifacts/<area>-<stamp>/`
(`result.json`, `gametest.log`). The latest full run (2026-10-10, revision 911b88d) passed 60/60.
The list of tests and what each asserts is in [Developer reference](developer.md#7-tests).
