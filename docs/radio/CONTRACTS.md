# Radio — frozen contracts (Phase 0)

Every radio lane builds against these. Changing one needs the integration agent.

## Java API (`com.example.evanscomputermod.radio.api`, pure)

| Type | Role |
| --- | --- |
| `Band`, `Channel` | Spectrum. `Channel.wifi24(n)`, `Channel.wifi5(n, width)`, `overlaps`, `wifiNumber` |
| `Pose` | World pose (dimension, x/y/z, quaternion). Sable endpoints report the projected world pose |
| `AntennaPattern` | Gain + polarization in the antenna's local frame; `ISOTROPIC`, `VERTICAL_DIPOLE` |
| `Emission` | `FRAME` / `IQ` / `ENERGY` on the airtime clock (µs) |
| `RadioEndpoint` | Anything that sends/hears; `onReceive(Reception)` is called off-thread |
| `RadioMedium` | `register`, `unregister`, `invalidate`, `transmit`, `channelPowerDbm`, `pathGainDb`, `nowMicros` |

`RadioMediumHooks.medium()` returns the server's medium (1.21.1). `BasicRadioMedium` is the reference implementation (free space + interference + PER). Lane 3A extends it via `extraPathLossDb` and installs itself with `RadioMediumHooks.setFactory`.

Modulation names used for PER (`BasicRadioMedium.requiredSinrDb`): `DSSS-1`, `DSSS-2`, `CCK-5.5`, `CCK-11`, `OFDM-6` … `OFDM-54`, `HT-MCS0..7`, `CHIRP-SFn`, `AFSK1200`, `FSK`, `BPSK`, `QPSK`, `CTRL` (wireless controller).

## Registration

- `RadioContent.register(modBus)` — one line per feature calling its own `*Content.register(modBus)`.
- `RadioScenarios` — one `add(...)` line per scenario. GameTests: namespace `ecm_radio`, structure `gametest_radio` (40×16×40), class `RadioTests` (add tests there or in sibling `Radio*Tests` with `@GameTestHolder(RadioTests.NS)`).
- Config: `RadioConfig` (SERVER). Add keys there.
- Tags: `rf_conductors`, `rf_insulators`, `rf_good_ground` (blocks), `rf_wrenches` (items).

## Registry IDs (frozen)

Blocks: `burner_generator`, `access_point`, `copper_wire`, `antenna_wire`, `heavy_cable`, `antenna_rod`, `lattice_mast`, `insulator`, `feed_point`, `coax_cable`, `hardline`, `lightning_arrestor`, `amplifier_100w`, `amplifier_1kw`, `amplifier_10kw`, `antenna_tuner`, `sdr_basic`, `sdr_standard`, `sdr_advanced`, `dish_small`, `dish_medium`, `dish_large`, `microwave_radio`.
Items: `wifi_module`, `controller_receiver_module`, `handheld_radio`, `antenna_analyzer`, `rf_meter`, plus block items. `sensor_wire` keeps its ID (display name "Fine Wire").

## Kernel host ABI additions (`abi/host-abi.toml`, module `env`)

Wi-Fi SoftMAC (kernel imports, provided by the Wi-Fi module through the computer):

| Function | Signature | Meaning |
| --- | --- | --- |
| `wifi_present` | `() -> i32` | number of Wi-Fi radios (0 = none) |
| `wifi_tx_frame` | `(ptr: i32, len: i32, rate_kbps: i32, power_dbm_x10: i32) -> i32` | queue a raw 802.11 frame (no FCS); 0 ok, <0 error |
| `wifi_rx_frame` | `(buf: i32, cap: i32, meta: i32) -> i32` | dequeue one frame; returns length or 0; `meta` = 24 bytes: rssi_dbm_x10 i32, rate_kbps i32, channel i32, timestamp_us i64, flags i32 (bit0 FCS ok) |
| `wifi_set_channel` | `(channel: i32) -> i32` | tune (802.11 channel number) |
| `wifi_set_rx_filter` | `(mode: i32, bssid_ptr: i32) -> i32` | 0 = own MAC + broadcast + BSSID, 1 = promiscuous, 2 = monitor |
| `wifi_get_mac` | `(out_ptr: i32) -> i32` | 6-byte MAC |

Received frames raise IRQ `wifi_rx` through the existing interrupt path.

## Sockets

`AF_PACKET` (17) with `SOCK_RAW` (3), protocol = EtherType (network byte order, `ETH_P_ALL` 0x0003), bound to an interface name; added to the socket syscalls in `net/ipc.rs` / `SocketFd` / `ecm-host-abi/src/socket.rs`.

Frozen details (lane 1A, implemented in `ecm_net::Stack::packet_*` and `net/ipc.rs`):

- Address `SockAddrLl`, 16 bytes so every host's sockaddr path carries it: `[family u16 LE = 17][protocol u16 network order][ifname 12 bytes, NUL-padded]`. `bind` names the interface (a non-zero protocol replaces the filter); `sendto` with a non-empty name sends out of that interface; `recvfrom`/`getsockname` return the arrival interface and the frame's EtherType.
- Frames are whole Ethernet frames without FCS (14..=1518 bytes), sent exactly as given. Received frames are copies: frames to the interface MAC, broadcast or any group address, while the interface is up, before the IP stack sees them (which still does). Outgoing frames are not looped back. 64 frames queue per socket; more are dropped.
- Unbound sockets cannot send or receive (-1). recv blocks (IPC_PENDING) unless `SO_RCVTIMEO` (-2 on timeout) or `MSG_DONTWAIT` (0x40; also honoured by recvfrom, which returns 0); recvfrom's default timeout is 5 s like UDP.
- `SOCK_POLL` = 15, child function `sock_poll(fds, n, timeout_ms) -> ready` over `pollfd {fd i32, events i16, revents i16}` (POLLIN 1, POLLOUT 4, POLLERR 8, POLLNVAL 0x20; at most 64; timeout <0 = forever). Mirrored in `WasiFunctions` and `rust/simulator/src/child.rs`.
- Kernel DHCP client control: private netlink `RTM_ECM_DHCP` (0x7E10) with `IFLA_ECM_DHCP_OP` start / release / status / stop (`ecm_host_abi::dhcp`).

## Device files (children)

`/dev/sdr<N>` (read/write interleaved samples; format from ctl), `/dev/sdrctl<N>` (text commands: `freq <hz>`, `rate <sps>`, `gain <db>|agc`, `bw <hz>`, `format cs16|cf32`, `tx on|off`), modelled on `/dev/audio.<name>` + `/dev/audioctl.<name>`.
