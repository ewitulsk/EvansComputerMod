# Wi-Fi

Part of the [Radio & Wireless guide](../RADIO_GUIDE.md).

Wi-Fi in the mod is real 802.11: byte-accurate frames, WPA2-PSK with the real 4-way handshake and
AES-CCMP encryption (checked against the IEEE test vectors), going over the shared radio medium
(so walls, distance and other radios matter). Two pieces of hardware:

- **Wi-Fi Access Point** (block): sits on a network cable and bridges wireless clients onto
  that cable.
- **Wi-Fi Module** (bay module): the computer's `wlan0`.

## 1. Quick start

1. Place an **Access Point** touching a **Network Cable** that reaches your wired computers.
   Right-click it, set an SSID, Security `WPA2-PSK (AES-CCMP)`, a passphrase (8–63 characters),
   **Apply**.
2. On another computer: right-click its side with a **Module Expansion Card**, then with a
   **Wi-Fi Module**.
3. Something on the cable must hand out addresses: run `dhcpd` on a wired computer (section 5),
   or give addresses by hand.
4. On the Wi-Fi computer:

   ```
   wifi scan
   wifi connect my-ssid "my passphrase"
   wifi
   ```

The Internet Gateway never serves DHCP; wireless clients reach the internet only if the AP's
cable network does (through a router or a computer with a route to the gateway).

## 2. The Access Point

### Placing and cabling

- Place it touching a Network Cable (or an Internet Gateway block). It joins the **first**
  touching cable segment in the order down, up, north, south, west, east; any face works but only
  one segment is joined. The LEDs (blockstate `active`) light when it has joined a segment.
- It is a **layer-2 bridge port** on that cable network: frames from wireless clients go onto
  the cable with the client's own MAC (the network learns the client is "behind the AP"), and
  frames on the cable for a client (or broadcasts) go out over the air. Wired frames are queued
  (up to 1024) and sent on the server tick. Two APs on one cable segment both bridge it.
- No power needed. Antenna: vertical omni, 5 dBi on the horizon, 0.3 block above the block
  centre; receive sensitivity −94 dBm.

### Who can change it

- The player who places it is its **owner**. Owner, operators (permission level 2), creative
  players, or anyone if it has no owner can open the settings; others get
  `This access point belongs to <name>`. You must be within 8 blocks (`Too far from the access point`).
- **Sneak + right-click with an RF Wrench** (or any `#c:tools/wrench`): factory reset. Settings
  return to defaults (open, SSID `ECM-XXXX` from the last two bytes of its BSSID, channel auto,
  20 dBm), the passphrase is erased, and **whoever reset it becomes the owner** (no permission
  check). Message: `Access point reset to factory settings (open network "ECM-6A43")`.

### Settings tab

| Control | Values |
|---|---|
| SSID | 1–32 bytes, no control characters |
| Hidden SSID | On: beacons carry an empty SSID and wildcard probes go unanswered (clients must probe for the name) |
| Security | `Open` or `WPA2-PSK (AES-CCMP)` (no WEP, TKIP, WPA3 or Enterprise) |
| Passphrase | 8–63 printable ASCII characters, or exactly 64 hex digits (the raw key). Masked with `*`. Leave empty to keep the stored one: the hint shows `(unchanged)` when one is stored, `(not set)` otherwise |
| Channel | `Auto (1/6/11)`, `1`, `6`, `11` (2.4 GHz), `36`, `40`, `44`, `48`, `149`, `153`, `157`, `161`, `165` (5 GHz). All 20 MHz |
| Transmit power | cycles 0, 3, 6, 10, 13, 15, 17, 20 dBm (shown as e.g. `20 dBm (100 mW)`) |
| Client isolation | On: clients can't talk to each other through the AP (they still reach the cable) |
| MAC filter | `Off`, `Allow only listed`, `Deny listed` |
| Filter list | up to 32 MACs, separated by commas, semicolons or spaces (`aa:bb:cc:dd:ee:ff, ...`) |

**Apply** validates on the server: `Settings applied`, or `Not applied: <reason>`
(`SSID must be 1-32 bytes`, `WPA2 needs a passphrase`,
`passphrase must be 8-63 printable ASCII characters or 64 hex digits`, `channel N is not offered`,
`transmit power must be 0-20 dBm`, `not a MAC address: X`). The passphrase field is cleared after
every Apply.

**The passphrase never leaves the server**: it is saved in the block's server data only; the
client, the GUI and block updates only learn whether one is set. A GameTest checks the update
tag, the data packet, the GUI view and the client copy.

**Auto channel** picks the least busy of 6, 1, 11 (ties in that order), counting what the AP
hears on each and the other Access Points nearby weighted by channel overlap. It is chosen when
settings are applied, the AP loads, or it is reset; it doesn't move later.

### Status tab

```
BSSID 02:a1:1d:05:6a:43   port 02:a0:...
Radio on, channel 6   cable connected   SSID ecm-room
Frames: 120 on air, 34 from cable, 28 to cable
Clients (1)      RSSI   rate   state / handshake
02:5e:...  -35 dBm  54M  authorized / done            [x]
1520 ms: 02:5e:... authorized
```

(layout from the code; the `wifi_room` scenario's check read `phone AUTHORIZED/DONE at -35 dBm;
rogue not listed`.) Up to 5 clients, each with RSSI, rate, state (`authenticated`, `associated`,
`authorized`), handshake (`none`, `ptk_start`, `ptk_negotiating`, `done`, `gtk_rekeying`), its last
error in red (e.g. `4-way M2 MIC mismatch (wrong passphrase?)`), and an **x** button
("Deauthenticate this client"). The last three events are listed at the bottom. `cable NOT connected`
is shown in orange. It refreshes every half second while open.

### What the AP does on the air

| | |
|---|---|
| Standards | 802.11g (2.4 GHz) / 802.11a (5 GHz): OFDM data rates 6–54 Mb/s; management and group frames at 1 Mb/s DSSS (2.4) or 6 Mb/s OFDM (5). **No 802.11n (HT)** |
| Beacons | every 102.4 ms (100 TU), sent on the server tick (at most one per 50 ms tick); DTIM period 1 |
| Probe responses | answered on the next server tick (up to ~50 ms) |
| Authentication | Open System only |
| Association | lowest free AID (up to 2007 clients); rejects wrong SSID, wrong RSN (cipher/AKM) |
| WPA2 4-way handshake | message 1/3 retried every 1 s, 4 tries, then deauth (reason 15). A wrong passphrase fails at message 2's MIC; the AP stays silent and the client eventually gives up |
| Group key | rekeyed every hour |
| Inactivity | a client that sends nothing for 300 s is deauthenticated (reason 4) |
| Unicast data rate | the fastest OFDM rate whose SINR need (4/5/7/9/12/16/20/21 dB for 6–54 Mb/s) is met with 3 dB margin, from the last RSSI (54 Mb/s needs about −71 dBm in quiet conditions) |
| ACKs | acknowledges every non-control frame addressed to it, one SIFS later, at 1 or 6 Mb/s |
| Retransmission | **none**: frames the client misses (for example while it scans another channel) are lost |
| Queue | up to 250 ms of airtime backlog; beyond that frames are dropped |

## 3. The Wi-Fi Module

- Install: Module Expansion Card on a computer side (not the screen), then the module on the same
  side. A computer has two bays (left and right of its facing), two slots each:
  `left_bay_1`, `left_bay_2`, `right_bay_1`, `right_bay_2`. The kernel uses the **first** module
  in Wi-Fi mode as `wlan0`.
- Antenna: vertical dipole (2.15 dBi) at the computer's centre; sensitivity −92 dBm; transmit
  0–20 dBm (`iw dev wlan0 set txpower fixed <mBm>`, capped by the peripheral's `set_max_power`).
- MAC: random, locally administered, saved with the module.
- Low MAC (in Java, under the kernel): appends/strips the FCS, ACKs frames addressed to it,
  retransmits unacknowledged frames up to 7 times (8 attempts) with exponential backoff (CW 15 to
  1023, 9 µs slots), carrier sense at −82 dBm, honours NAV. The kernel picks rates (minstrel-style
  rate control on transmit statuses).
- Modes (`set_mode` on the `wifi` peripheral, or `wifi mode wifi|controller`): `wifi` (default)
  or `controller` (a Wireless Controller receiver on channel 1–13, default 6; `wlan0` disappears).
- Peripheral methods: [Programming](programming.md#wi-fi-module-wifi).

## 4. Inside the computer: wlan0 and the Wi-Fi control protocol

The kernel runs the 802.11 station (`rust/crates/ecm-wifi`): scanning, authentication,
association, the WPA2 supplicant's key installation, encryption, roaming. `wlan0` appears when a
Wi-Fi Module is present (checked every second; hot-plug works), and lines for `wlan0` in the
computer's network configuration are applied then. Its link is up while associated (data other
than EAPOL waits for the keys).

Station behaviour:

| | |
|---|---|
| Scan | channels **1–13 only**, active, 120 ms per channel (≈ 1.6 s total); passive 110 ms |
| Authentication / association | 200 ms timeout, 3 tries |
| Beacon loss | after 1 s without beacons it probes the AP; after 2 s it disconnects (`beacon_loss`) |
| Reconnect | automatically after 1 s |
| Roaming | when the signal drops below −70 dBm it scans (at most every 10 s) and reassociates to another AP with the same SSID if it is 8 dB stronger (tested only in simulation) |
| Security | Open and WPA2-PSK/CCMP. Group-key rekeys handled. No TKIP, WPA3, 802.11w (MFP), PTK rekey, power save or fragmentation |
| Rates | 1–11 Mb/s DSSS/CCK and 6–54 Mb/s OFDM; it advertises HT but never uses it (the AP has none) |

Programs talk to it through `WIFI_CTL` (IPC syscall 48; `ecm_host_abi::wifi::ctl(text)` in Rust).
One text request per call, reply up to 8188 bytes; errors are negative statuses
(`-19 no Wi-Fi module`, `-16` busy, `-22` invalid, `-107` not associated):

| Request | Reply / effect |
|---|---|
| `status` | `present=1`, `ifname=wlan0`, `mac=`, `mode=managed|monitor`, `state=IDLE|SCANNING|AUTHENTICATING|ASSOCIATING|ASSOCIATED|DISCONNECTED|MONITOR`, `scanning=`, `channel=`, `freq=`, `tx_power_dbm=`, `authorized=`, `ptk=`, `scan_results=`; when associated `bssid=`, `ssid=<hex>`, `bss_channel=`, `signal=`; `event_seq=` (or just `present=0`) |
| `events <after_seq>` | lines `<seq> <NAME> k=v...`: `PRESENT`, `REMOVED`, `SCAN_DONE results= aborted=`, `CONNECTED bssid= ssid= channel= aid= ap_rsn= sta_rsn= roamed=`, `CONNECT_FAILED bssid= reason=no_bss|auth_timeout|auth_rejected:N|assoc_timeout|assoc_rejected:N|deauth:N`, `DISCONNECTED bssid= reason= kind=deauth|disassoc|beacon_loss|local local=` |
| `scan [ssid_hex] [passive]` | start a scan (`-16` if one is running or in monitor mode) |
| `scan_results` | `bssid freq signal_dbm open|wpa2-psk|unsupported channel ssid_hex` lines, strongest first, seen in the last 30 s |
| `connect <ssid_hex> open|wpa2 [bssid]` | join |
| `disconnect [reason]` | leave (default reason 3) |
| `link` | the `iw link` text, or `Not connected.` |
| `set_type monitor|managed|station`, `set_channel <n>`, `set_power <dbm>` | |
| `install_ptk <bssid> <tk_hex>`, `install_gtk <id> <rsc> <gtk_hex>`, `clear_keys` | used by `wpa_supplicant` |
| `eapol_subscribe`, `eapol_unsubscribe`, `eapol_rx`, `eapol_tx <eth_hex>` | EAPOL tap |
| `mon_read` | next monitor-mode frame with a radiotap header |
| `stats` | `rx_packets, rx_bytes, tx_packets, tx_bytes, tx_retries, tx_failed, rx_dropped_replay, rx_dropped_mic, rx_duplicates, monitor_queued, monitor_dropped` |

The raw frame ABI between kernel and module (`wifi_tx_frame`, `wifi_rx_frame`, `wifi_set_channel`,
`wifi_set_rx_filter`, `wifi_tx_status`, IRQ 5) is in `docs/radio/CONTRACTS.md`.

## 5. Programs

### `wifi`: one command to join

```
wifi                          show the Wi-Fi connection
wifi scan                     list networks in range
wifi connect <name> [password] join a network (quote names/passwords with spaces)
wifi disconnect               leave the network
wifi mode wifi|controller     use the Wi-Fi Module for Wi-Fi or as a Wireless Controller receiver
```

`wifi connect` checks every requirement in order and stops with the problem and how to fix it:

| Message | Meaning |
|---|---|
| `wifi: this computer has no Wi-Fi Module` + the expansion card steps | no module |
| `wifi: the Wi-Fi Module is in controller mode ...` → `Run: wifi mode wifi` | |
| `wifi: wlan0 is in monitor mode (capturing packets), so it can't join a network` | |
| `wifi: no Wi-Fi networks are in range` | empty scan |
| `wifi: no network called 'X' is in range (this computer hears 'A' (-50 dBm), ...)` | with a "did you mean" for case differences |
| `wifi: 'X' needs a password (WPA2)` | |
| `wifi: that password can't be right: WPA2 passwords are 8 to 63 characters, this one is N` | |
| `wifi: 'X' uses a kind of security this computer can't join` | |
| `wifi: the password for 'X' is wrong (the Access Point rejected it)` | 4-way handshake failed |
| `wifi: the Access Point for 'X' refused this computer (...)` | 3 association rejections (MAC filter, full) |
| `wifi: couldn't join 'X' in 20 seconds (signal N dBm, last step: STATE)` | |
| `wifi: joined 'X', but nothing gave this computer an address (no DHCP server answered in 10 s; DHCP state S)` | fix: run `dhcpd &` on a wired computer, or `ifconfig wlan0 <address>/<prefix>` |

Real output (laptop in the `wifi_connect` scenario):

```
/ > wifi connect ecm-cafe letmein123
wifi: this computer has no Wi-Fi Module
To fix it:
  1. Hold a Module Expansion Card and right-click a side of the computer (not the screen) to fit a bay.
  2. Hold a Wi-Fi Module and right-click the same side to put it in the bay.
  ...
/ > wifi connect ecm-cafe wrongpass1
Looking for 'ecm-cafe'...
Found 'ecm-cafe' (-30 dBm, channel 6, WPA2). Joining...
wifi: the password for 'ecm-cafe' is wrong (the Access Point rejected it)
...
/ > wifi connect ecm-cafe
wifi: 'ecm-cafe' needs a password (WPA2)
To fix it:
  1. Run: wifi connect ecm-cafe <password>
```

On success it prints `Joined 'X' (<bssid>, channel C, N dBm). Getting an address...` and
`Connected. Address 192.168.60.10/24, router 192.168.60.1, DNS 1.1.1.1`. Notes it may add: the
password is ignored for an open network; `The signal is weak (N dBm)` below −80 dBm.

What it does: stops a running `wpa_supplicant` on wlan0 (`Stopping the running wpa_supplicant so
wifi can manage wlan0...`), saves the network into `/etc/wpa_supplicant.conf` (replacing an entry
with the same SSID), joins and completes the handshake itself, then starts the kernel's DHCP
client on wlan0 (persisted). It exits once connected; the kernel keeps the association.

`wifi` (no arguments): `wlan0: connected to 'ecm-cafe' (<bssid>)`, `  channel 6, signal -30 dBm, WPA2`,
`  address: 192.168.60.10/24` (or `wlan0: not connected (state S)` with a hint).
`wifi scan`: a `NETWORK  SIGNAL  CH  SECURITY` table (`open`, `WPA2 (password)`, `unsupported`,
`(hidden)`), or `No Wi-Fi networks in range.`. `wifi disconnect`: `wlan0: disconnected`.

**Caveat**: `wifi connect` does not leave a supplicant running. If the link later drops (beacon
loss, the AP's 300 s inactivity timeout on an idle computer, an AP restart), the kernel
reassociates but nothing answers the AP's handshake, so the connection stays down until you run
`wifi connect` again. For a computer that must stay connected, run `wpa_supplicant -B` instead
(below), which re-handshakes automatically. (Inferred from the code; not covered by a test.)

### `iw`

```
iw dev                                    list wireless interfaces
iw dev wlan0 info                         interface details
iw dev wlan0 scan [passive] [ssid S]      scan for access points (scan dump: cached)
iw dev wlan0 link                         current link: signal and bitrate
iw dev wlan0 station dump                 counters
iw dev wlan0 set type monitor|managed     (station = managed)
iw dev wlan0 set channel <1-13|36-165>    (also: set freq <MHz>)
iw dev wlan0 set txpower fixed <mBm>      (or auto = 20 dBm)
iw dev wlan0 connect <ssid> [bssid]       join an open network
iw dev wlan0 disconnect
```

Channel 14 (`set channel 14`, `set freq 2484`) is refused with `command failed: bad channel (-22)`:
the radios have no channel 14.

Real `iw dev` output:

```
phy#0
        Interface wlan0
                ifindex 0
                wdev 0x1
                addr de:4e:57:05:3a:ed
                type managed
                channel 1 (2412 MHz), width: 20 MHz
                txpower 20.00 dBm
```

Scan entries: `BSS <bssid>(on wlan0)`, `freq:`, `signal: -42.00 dBm`, `SSID:`,
`DS Parameter set: channel 6`, and for WPA2 an `RSN:` block (`Version: 1`, `Group cipher: CCMP`,
`Pairwise ciphers: CCMP`, `Authentication suites: PSK`). `scan ssid S` sends a directed probe:
the way to find a hidden network. `link` (format from the code, example values):

```
Connected to <bssid> (on wlan0)
	SSID: ecm-lab
	freq: 2437
	RX: n bytes (n packets)
	TX: n bytes (n packets)
	signal: -45 dBm
	rx bitrate: 54.0 MBit/s
	tx bitrate: 54.0 MBit/s
	bss flags: short-slot-time
	dtim period: 1
	beacon int: 100
```

Errors: `command failed: No such device (-19)` (no module), `command failed: Operation not
supported (-95)` (scan in monitor mode), `command failed: scan timed out (-110)`.

### `wpa_supplicant` and `/etc/wpa_supplicant.conf`

```
usage: wpa_supplicant [-B] [-d] -i <ifname> [-c <config>] [-f <logfile>] [-D packet|ctl]
  -B  run in the background (the shell returns to the prompt)
  -c  configuration file (default /etc/wpa_supplicant.conf)
  -i  interface (wlan0)
```

The long-running supplicant: picks the best configured network in range, joins, does the WPA2
handshake over an AF_PACKET socket (EtherType 0x888E; `-D ctl` uses the kernel's EAPOL tap),
reconnects after drops, handles group rekeys. `-d` logs control requests; `-f` writes a log file.

Config file:

```
network={
    ssid="my-ssid"
    psk="my passphrase"
}
network={
    ssid="cafe"
    key_mgmt=NONE
    priority=5
}
```

Keys: `ssid="text"` or hex, `psk="8..63 chars"` or 64 hex digits, `key_mgmt=NONE|WPA-PSK`,
`bssid=`, `priority=`, `disabled=1` (`proto`, `pairwise`, `group`, `scan_ssid`, `id_str`,
`auth_alg` are accepted and ignored). Errors name the line:
`network block needs psk= or key_mgmt=NONE`, `unknown network variable 'X'`, ...

Log lines: `Successfully initialized wpa_supplicant (N networks)`,
`wlan0: Trying to associate with SSID 'X'`, `wlan0: Associated with <bssid>`,
`CTRL-EVENT-CONNECTED - Connection to <bssid> completed [id=0]`,
`CTRL-EVENT-DISCONNECTED bssid=<bssid> reason=15`,
`WPA: 4-Way Handshake failed - pre-shared key may be incorrect (<bssid>)`,
`WPA: Group rekeying completed (key id 2)`.

It keeps `/run/wpa_supplicant.wlan0.status` (`wpa_state=...`, `address=`, `bssid=`, `ssid=`...)
for `wpa_cli`, and reads commands from `/run/wpa_supplicant.wlan0.cmd`.

### `wpa_cli`

`status`, `scan` (`OK`/`FAIL-BUSY`), `scan_results`, `list_networks`, `add_network` (prints the new
id), `set_network <id> <var> <value>` (bare ssid/psk values are quoted for you), `get_network`
(psk shows `*`), `enable_network` / `disable_network` / `select_network` / `remove_network <id|all>`,
`save_config`, `reconfigure`, `disconnect`, `reconnect`, `reassociate`, `terminate`.
Edits are saved to the config file immediately. Failures print `FAIL`.

```
wpa_cli status         -> wpa_state=COMPLETED, address=..., bssid=..., ssid=..., key_mgmt=WPA2-PSK ...
wpa_cli scan_results   -> bssid / frequency / signal level / flags / ssid
                          02:a1:...  2437  -42  [WPA2-PSK-CCMP][ESS]  ecm-lab
wpa_cli list_networks  -> 0  ecm-lab  any  [CURRENT]
```

Joining by hand (what the scenarios type):

```
ifconfig wlan0 192.168.77.2/24
wpa_cli add_network
wpa_cli set_network 0 ssid ecm-lab
wpa_cli set_network 0 psk "correct horse battery"
wpa_cli enable_network 0
iw dev wlan0 scan
wpa_cli scan_results
wpa_supplicant -B -D packet -i wlan0 -c /etc/wpa_supplicant.conf
wpa_cli status                    (repeat until wpa_state=COMPLETED)
ping 192.168.77.1
```

### `dhcpd` and `/etc/dhcpd.conf`

```
Usage: dhcpd [-c config] [-l leasefile] [-t] [iface ...]
```

Defaults: `/etc/dhcpd.conf`, leases in `/var/dhcpd.leases`. `-t` checks the config. Run it in the
background: `dhcpd &`.

```
# one pool per interface
pool eth0 192.168.50.10 192.168.50.100 router 192.168.50.1 dns 1.1.1.1 lease 3600
reserve 02:5e:11:22:33:44 192.168.50.5
server eth0 192.168.50.1
```

`pool <iface> <first> <last> [prefix <n>|netmask <mask>] [router <ip>] [dns <ip>] [lease <s>]`
(prefix 24 and lease 3600 by default, lease ≥ 4 s), `reserve <mac> <ip>`, `server <iface> <ip>`
(the server identity; else the interface's address, else the router). The interface needs an
address in the pool's subnet. Output: `dhcpd: serving eth0 192.168.50.10-192.168.50.100/24 as 192.168.50.1 (lease 3600s)`,
then `DHCPDISCOVER from <mac> via eth0`, `DHCPOFFER on <ip> to <mac> via eth0`,
`DHCPREQUEST ...`, `DHCPACK on 192.168.50.10 to <mac> via eth0`, `DHCPRELEASE of ...`.

A computer serving DHCP for Wi-Fi clients runs it on its **cable** interface (eth0): the AP
bridges the clients onto that cable. The router service's own `dhcp-server` also works.

### `dhclient`

```
Usage: dhclient [-t secs] <iface> | -r <iface> | -x <iface> | -s <iface>
```

`dhclient eth0` (or `wlan0`) starts the kernel's DHCP client and waits up to 15 s (`-t`):
`bound to 192.168.50.10/24 -- renewal in 1800 seconds.` and `  router 192.168.50.1, dns 1.1.1.1, server ...`;
on timeout `dhclient: no lease on eth0 after 4s (state SELECTING); still trying in the background`.
`-s` status (`eth0: state BOUND`, lease, renew/rebind times), `-r` release, `-x` stop.
`ifconfig <iface> dhcp` uses the same client.

### `tcpdump -i wlan0` (monitor mode)

```
iw dev wlan0 set type monitor
iw dev wlan0 set channel 6
tcpdump -i wlan0 -c 10 -w cap.pcap
```

Flags: `-c N`, `-w FILE` (pcap, link type 127 = 802.11 + radiotap; opens in Wireshark), `-x`,
`-v`. Real output (`wifi_monitor`):

```
tcpdump: listening on wlan0, link-type IEEE802_11_RADIO (802.11 plus radiotap header), channel 1, snapshot length 65535 bytes
00:26:02.601020 1.0 Mb/s 2412 MHz -34dBm signal Probe Request () SA:1e:60:dc:b5:9c:12 DA:ff:ff:ff:ff:ff:ff
3 packets captured
```

Frames shown: beacons, probes, (re)association, authentication, deauth/disassoc, ACK/RTS/CTS,
data (`Data IV (CCMP)` when encrypted). Radiotap fields: timestamp, flags, rate, channel, signal
dBm. It does **not** decrypt WPA2 traffic. Without monitor mode:
`tcpdump: wlan0: not in monitor mode (run: iw dev wlan0 set type monitor)`. Return with
`iw dev wlan0 set type managed`.

## 6. Range and speed

There is no range setting: see [How radio travels](propagation.md). Measured in the `airship_radio`
test (AP at 0 dBm on the ship, phone computer in a wooden shack): `iw dev wlan0 link` read
−75 dBm at 14 m (the medium predicted −71.1) and −85 dBm at 40 m (predicted −81.0); at 500 m the
prediction is −136.5 dBm against the phone's −92 dBm sensitivity: no link. At 20 dBm add 20 dB.
One stone block costs ~66 dB at 2.4 GHz, so walls of stone end it; glass, wood, leaves and wool
cost a few to 14 dB.

## 7. Wi-Fi limitations

- **5 GHz is effectively unusable by computers**: the station only scans channels 1–13, so
  `wifi`, `wpa_supplicant` and `iw scan` never find a 5 GHz AP. (`iw dev wlan0 set channel 36`
  then `iw dev wlan0 connect <ssid>` within 3 s might join an open 5 GHz AP; untested.)
- No 802.11n/ac rates, no power save, no MFP, no PTK rekey, no WPA3/Enterprise/WEP/TKIP.
- The AP never retransmits; clients scanning (including automatic roam scans when the signal is
  below −70 dBm) can lose downlink frames.
- Hidden networks: `wifi connect` and `wpa_supplicant` scan with wildcard probes, so they don't
  find hidden SSIDs unless an `iw dev wlan0 scan ssid <name>` within the last 30 s has.
- A 64-hex-digit raw key is accepted by the AP and `wpa_supplicant` (`psk=` unquoted) but
  `wifi connect` rejects it (length 64).
- Idle clients are dropped after 300 s; only a running `wpa_supplicant` reconnects by itself.
- Roaming and 5 GHz are not tested in a running world.
- `iw`, `wpa_supplicant`, `wpa_cli` and `tcpdump` aren't listed by `help`.
