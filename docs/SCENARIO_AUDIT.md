# Scenario audit: real player procedures

AGENTS.md: every `/ecm scenario` is the procedure a player follows. Blocks are
placed and used by `ScenarioPlayer` through the game's interaction code, real
computers run real programs, and input arrives the way a player gives it. Reading
internal state to *check* a result is allowed. *Causing* a result through internals,
virtual stand-ins or cheated sources is not. The only standing exception is
scripted airship or vehicle motion. Any other exception is listed in
[SCENARIO_EXCEPTIONS.md](SCENARIO_EXCEPTIONS.md).

Audit of branch `gui/scenario-audit` (from feature/gui-engine @ 59f99e3), covering 58
catalog scenarios plus the `router_ring_tap_*` family that RouterTests and the
tech client suite build.

## Violations that affected every scenario (fixed in the framework)

| Violation | Was | Now |
|---|---|---|
| Typing | `TerminalBlockEntity.onStringInput` on the server, with no sender and no range check | `ScenarioPlayer.type/press/enter/ctrl/chord` build the client's packets (`TerminalInputPacket`, `KeyInputPacket` key and text) and run their server handlers (new `deliver(ServerPlayer, packet)`), so `acceptsInput` and the range rules apply. The player walks up to the screen first (`sitAt`). Several pasted lines are one Ctrl+V input (`Builder.paste`). |
| GUI keys, clicks and text | `ComputerInstance.handleKeyEvent/handleTextEvent/handlePointerInput`, `UiManager.pointer` and `Compositor.pointer/text` called directly | `ScenarioRun.pressKey/typeText/click` and `ScenarioPlayer.tap` send `KeyInputPacket`/`MouseInputPacket` through their handlers, from the terminal's player or the phone's holder |
| Non-player builds | `ScenarioRun.build` used `setBlock` for terminals and cables and booted with `initializeWasm` unless `asPlayer()` was set (switch labs, router labs, `gui_basics`, `gui_client_render`, `lidar_room`) | On 1.21.1, `ScenarioPlayer` builds and boots every scenario (it right-clicks the screens), and `Cut`/the new `Mend` step break and replace cables by hand. 26.1 has no scenario player and still sets blocks. |
| Screens | NeoForge's `FakePlayer` refuses every menu, so "open the screen, press Apply" never had a screen open (integrator note) | `ScenarioPlayer`'s hands open menus for real: the block's menu becomes the open container. The Access Point's Apply runs the Configure packet's handler while its screen is open, then closes it. The cell tower is out of scope (being rewritten). |
| `/ecm scenario link(s)` | An admin command that flipped logical patch leads | Removed, along with the leads |

## Per scenario

Codes for **steps**: P = player-real, C = check-only (allowed), E = kept as a documented
exception, X = violation fixed here, O = violation in another lane's file (not fixed).

| Scenario | Violations found | Fix / exception |
|---|---|---|
| switch_basic, switch_vlans, switch_trunk, switch_stp, switch_lacp | X: setBlock terminals and cables, `initializeWasm` boot, setBlock cable cut, typing through internals | Built, booted, cut and typed by the player (framework) |
| router_home, router_port_forward, router_static, router_wan_dhcp, router_bgp_pair, router_bgp_policy | X: setBlock build; logical patch leads (`CableNetworkManager.logicalLink`, virtual cables); startup files (`router.cfg`, `network.cfg`, `services.cfg`, `index.html`) written to disk | Real cables between the actual ports (eth0 bottom, eth1 top, eth2 south). The configuration is typed: `ifconfig` and `ip route add`, the services' commands, `router on`, and the router config pasted into the CLI and saved with `write memory`. Pages are typed from Python and `dhclient` runs last. |
| router_bgp_ring | Same, plus the ring cut through `/ecm scenario link ringN down` (logical) | Real 10-router ring. The cut breaks a block of the r10-r1 cable and of the r1-r2 cable, and the repair (`Mend`) puts them back. |
| router_chat | Same; the two leads on one port were a hub with no block | The server's eth0 is cabled to alice and its eth1 to bob (two LANs). chat.conf is typed. |
| router_internet | X: logical link to the internet proxy MAC; `enableInternetProxy()` called; network.cfg and the URL file written to disk; `sendInput` used to run curl | The player places an Internet Gateway block and cables it. The address and route are typed, and `curl <url>` is typed, with the URL being the host web server's address. C: a check that the segment is online. |
| router_fiber | X: setBlock fiber spans and patch panel, `ModuleBays.installCard/installModule`, `removeBlock` | Spans and the panel placed by clicks, the Always-On Module clicked in, the arm broken and replaced by the player. C: NBT round-trip check. |
| router_headless | X: module installed through internals; `sendInput("sleep 1 &")` into a headless computer | Module clicked in; `sleep 1 &` typed before leaving. E: chunk unload and load hooks. |
| router_village | X: setBlock build; worldgen configs written to disk | Village 5's configs typed (chat.conf left out, since the lab doesn't chat) |
| router_player_fiber | none (already player-built) | — |
| router_fiber_tap | X: startup files written to disk | Typed as above |
| router_ring_tap_* (RouterTests, tech client suite) | X: the PC's network.cfg written to disk. E: commands typed straight into the far village ISP router's console (`ispShows`). E: chunk unload and load hooks. | PC configuration typed. Exceptions noted in the steps. |
| lidar_room | X: lidar set with setBlock, module put in through `ModuleBays`, wire made with `SensorWireItem.connect` on imaginary endpoints, `lidar_view.py` written to disk | Lidar placed on the floor, card and module clicked in, and Sensor Wire clicked from connector to connector through the wire item's click handler. The program is typed from Python (82 lines). |
| dhcp_lan, wifi_room, wifi_monitor, wifi_wpa2_ping, wifi_connect, ham_dipole, antenna_tools, sdr_lab, radio0_lab, microwave_link, wifi_walls | Typing (framework). C: `/ecm radio link` is a diagnostic read; reading the far dish's pose to type `aim_at` uses the same numbers its `info()` shows. Chests are filled like a creative player does, which is vanilla and allowed. | Framework fix only |
| wifi_5ghz | E: the GameTest hook `setInactivityTimeoutForTest(5000)` | Kept, noted in the scenario and in the exceptions |
| ham_station | none beyond typing; `/gamerule` and `/summon` are operator commands | — |
| radio_station | E: WAV files copied into the computer folder. E: a handheld receiver session with no holder (`HandheldServer.Session.receive`). | Kept, noted in the steps |
| airship_radio_range, airship_radio_return (was airship_radio) | Standing exception: scripted assembly and flight. `/forceload` is an operator command. Hull frame and envelope blocks are set like terrain as part of the ship. | — |
| cell_lte_far, cell_mmwave_hill, cell_handover, cell_wrong_carrier | O (cellular lane, `CellularScenarios.java`): tower screens are driven by `CellPackets.act` (tower being rewritten); `removeModem` clears the player's inventory directly. Allowed: `/forceload` and `remoteHost` (the player travels 300 blocks to build and type), Sable cart motion (standing exception). | Rewritten in Cellular v2 phase 4 (see "Cellular v2 phase 4" below). |
| panel_swap, panel_wall, panel_touch | Typing (framework) | — |
| gui_basics | X: setBlock build, SDR set with setBlock, `/etc/ui.toml` written to disk, `ProcessManager.kill` on the shell, the console chord through `handleKeyEvent`. E: a malformed UI connection opened by the scenario. | Player-built; SDR placed; ui.toml typed. The shell is killed from the console (Ctrl+Alt+Backspace, `ps`, `kill <pid>`). E: arm.gba copied in; the garbage-client control is kept. |
| gui_desktop | X: dhcpd.conf, ui.toml and notes written to disk; keys, text and clicks through internals; the clipboard control was a connection the scenario injected; the permission prompt was `Compositor.requestPermission` on a fake app | Files typed. The pc is restarted by breaking it, picking it up and placing it back, which is how autostart is reached. The control is now a real program (`python /home/snoop.py`, no window), typed into the GUI Terminal. The prompt comes from a real app: `store --sample-repo`, `pkg install <file>` and `pkg run dev.ecm.netcheck` in the Terminal, then Deny. |
| gui_client_render | X: setBlock build, files written to disk, `Compositor.pointer/text`, `UiManager.control("launch gba")`, a System property toggle | Typed and clicked through packets. gba is started by typing `gui launch gba arm.gba` at the console. `ui.clientRendering` is set in the server config. E: the headless client (`SceneViewerMirror`, display key 9). E: arm.gba copied in. |
| app_repo | Typing (framework) | — |
| app_store | X: `UiManager.pointer` clicks, chord and Alt+F4 through `handleKeyEvent` | Packets |
| files_open_with | X: hello.py and secret.py written to disk; keys and text through internals | Typed from Python; packets |
| visual_console, visual_app | X: the chord through `handleKeyEvent` | Packets |
| phone_basics, phone_visibility, phone_wifi_walk | X: count.py written to disk. X (follow-up): programs typed into the phone's console behind its GUI (`consoleInput`); now typed in the phone's built-in Terminal app, opened by a tap on the home grid, with the holder's key and text packets. E: a second player (`Onlooker`). Allowed: server-config overrides (`PhoneConfig.override`) and display prefs set through `PhoneServer.setPrefs`, which does what the Prefs packet handler does. | count.py typed in the Terminal app. Exceptions noted. |
| phone_battery | X: `setBatteryForTest(0.5)` (a cheated source) | The battery really drains: the operator sets `battery.awakePerMinute = 6000` |
| phone_duplicates | E: second player (`Twin`); a fake player can't use a portal, so the Nether trip is a direct move | Noted |
| phone_gui, phone_background, phone_gba | X: taps, Alt keys and text through `ComputerInstance` (after a `resolveInput` check). E: probe apps copied in, permissions pre-granted, refresh interval compressed, ROM copied in. | Taps, keys and text are the holder's packets. Exceptions noted. |

Counts: 58 catalog scenarios. 31 had violations that were fixed beyond the
framework-wide input, build and screen fixes. 12 keep documented exceptions, and 4
(cellular) are in another lane's file. Every scenario benefited from the framework
fixes.

## Found along the way

- `terminal_block` was missing from `minecraft:mineable/pickaxe` while requiring the
  correct tool, so a survival player who broke a computer lost it (no drop). The
  power-cycle step found it. Fixed in the tag.
- Python starts in 4 to 6 s in GameTests: "Chicory compiler unavailable, programs will be
  interpreted: NoSuchMethodError Remapper.<init>(int)" (an ASM version clash on the
  run classpath). Typed files are batched into one REPL per computer because of it.
  Fixed on `gui/followups`: the compiler ships with its own relocated ASM
  (docs/refactor/ARCHITECTURE.md, "Chicory's compiler and ASM").
- Shell redirects (`echo text > file`) don't create files from programs, so a player
  can't write a file with echo. Python's `shell.write_file` was used instead.

## Follow-up re-audit (branch `gui/followups`)

Scenarios written in parallel with the audit, checked against the same rule.
`CellularScenarios.java` is being rewritten and was not touched.

| Scenario | Found | Fix / exception |
|---|---|---|
| phone_basics, phone_visibility, phone_wifi_walk | E (was): programs typed into the kernel console behind the phone's GUI | The ePhone has a built-in **Terminal** app (`dev.ecm.terminal`, home grid). The scenarios tap it open and type through the holder's `KeyInputPacket`s (text, Enter, Ctrl+T). Output is read back from the Terminal window's drawn text (a check). The exception is removed. |
| phone_dock | X: the phone's battery set to 50 % in the item (`PhoneItemData.setBatteryPercent`), a cheated source | The operator sets `battery.awakePerMinute = 1200` (server config) and the held phone drains below 90 %, then back to 1. C: the client-side dock copies fed the update packet and update tag through the client's own handlers (`onDataPacket`, `handleUpdateTag`), and the empty-tag control, only read state. |
| phone_boot_together | none: Terminals placed and opened by the player, the phone used and moved to the off hand | — |
| chat_app | X: text typed into the client's GUI with `ComputerInstance.handleTextEvent` | `ScenarioRun.typeText` (the player's `KeyInputPacket` text through its handler). Clicks were already `MouseInputPacket`s. C: `AppManager.manifest` checks. |
| phone_gba (GBA steps) | X: Space pressed with `ComputerInstance.handleKeyEvent` (Settings > Controls rebinding) | The holder's key packets (`ScenarioPlayer.chord`). Allowed: hotkeys through the hotkey packet's handler, the controller's input through `ControllerServer.deliverInput` (its packet handler), pairing by right-clicking the controller. E (kept): the ROM copied in, controller and speaker permissions pre-granted. |
| gui_close_over_overlay | Not a catalog scenario: a case of the `-ClientChecks -ClientSuite gui` suite. Its step is a real click through Minecraft's mouse handler on the real client, with a click-eating overlay installed as the control. The suite's server fixture switches the Terminal to the desktop shell by writing `/etc/ui.toml` and restarting the UI; that is test-fixture setup, not a player procedure. | — |
| airship_radio (split) | It passed alone in 59 s against its 60 s limit and timed out under load ("ping holds at 40 m" alone took 18 to 36 s: each lost `ping -n 1` cost a 5 s timeout). | Split into `airship_radio_range` (in-place ping, assembly, pings from the air and at 14 m and 40 m, RSSI and rate falling as the medium predicts) and `airship_radio_return` (500 m beacon loss with the medium's prediction under sensitivity and no ping reply, re-association at 14 m, landing with AP settings, passphrase and cable segment intact). Every check and control is kept; the standing exception is still scripted flight. Found on the way: the slow 40 m ping was the Access Point sending unicast data at the rate picked from the phone's RSSI at 14 m (the phone only listens, so it wasn't heard again until it happened to transmit). `WifiAirLink` now uses the basic rate for a peer not heard for 2 s. Alone: range 15 to 23 s, return 29 to 31 s (two runs each). |

## Cellular v2 phase 4 (branch `gui/cell2-cleanup`)

The v1 cellular scenarios (Cell Tower block, tower screen driven by `CellPackets.act`, the v1 modem engine pinned by
`V1_ENGINE`) were deleted with v1 and rewritten on player-built networks (docs/gui/CELLULAR_V2.md, "As built: phase 4").

| Scenario | Checked | Fix / exception |
|---|---|---|
| cell_lte_far | Built and typed by the player: core (router CLI, `cored`), server, site (`celld-site`, SDR, coax mast, LTE panel) 300 blocks away; the SIM written by `cored sim new` at the far reader and clicked into the phone's modem bay. C: the antenna's distance from the modem's pose, `cored status`. | Allowed: `/forceload` and `remoteHost` (the player travels 300 blocks to build and type) |
| cell_mmwave_hill | Built, typed and clicked: two sites, SIMs, `celld &`. The modem swap is the player's: `ps` + `kill`, sneak-right-click the bay (the modem comes out), sneak-use the loose modem (its SIM comes out), the LTE Modem and the SIM clicked in. C: the traced path losses (`traceNow`) and the modems' poses. | — |
| cell_wrong_carrier | The blue SIM is written by the player with `cardtool sim` at the same card writer; the red one by `cored sim new`; both clicked into the bay. C: `celld-site status`, `cored log`, the returned card's key. | — |
| cell_roaming (was cell_handover) | Seven player-built computers (two sites, two routers, core, server, phone); all routing typed (router CLI, `ip route add`). | Standing exception: the cart is a Sable ship moved by script. Cart planks set like terrain. |
| cell_chat | Network as cell_phone_attach; the server packages and serves Chat itself; the phone's App Store, Chat, its permission dialog are tapped; text typed with the holder's key packets; `pkg perm grant` typed in the phone's Terminal; server lines typed at its keyboard (the phone moves to the off hand meanwhile). C: `AppManager.manifest`, the permission store, the compositor's notifications. | E: the background refresh interval compressed (minutes to 3 s) for the run, as in phone_background |

The catalog renames cell_handover to cell_roaming and gains cell_chat; the four cellular scenarios that were "in
another lane's file" are now audited, and one (cell_chat) keeps a documented exception.
