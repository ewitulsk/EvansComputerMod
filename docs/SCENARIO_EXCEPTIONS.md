# Scenario exceptions

AGENTS.md: a step that genuinely can't be a player-equivalent action is written in
the scenario's notes and here. The standing exception, scripted airship and vehicle
motion (`airship_radio_range` and `airship_radio_return`, the `cell_roaming` cart), needs no entry. Everything else:

| Exception | Scenarios | Why it can't be player-real | Notes |
|---|---|---|---|
| Binary files copied into a computer's folder (`<world>/computer-data/<id>/`) | gui_basics, gui_client_render (arm.gba); phone_gba (arm.gba into the phone's /home); radio_station (WAV songs) | Binary data can't be typed. A player brings ROMs and audio in from outside the game, and copying into the computer's folder is the documented way. | Text files are always typed (Python `shell.write_file`) |
| A malformed UI client opened by the scenario (`Compositor.open` plus an unknown opcode) | gui_basics (control) | No program a player can run sends malformed protocol bytes: the Python `ui` module only sends well-formed messages. | Checks that a broken client loses only its own connection |
| A headless client that draws scenes (`SceneViewerMirror`, display key 9) | gui_client_render | A GameTest server has no client. The real client is covered by `-ClientChecks -ClientSuite gui` (case `gui_client_render`). | All input in the scenario comes from the scenario player through packets |
| Chunk unload and load hooks called directly (`onChunkUnloaded`, `onLoad`) | router_headless, router_ring_tap_*_cut | A GameTest can't unload a test's chunk by walking away | The computers keep running headless; nothing is typed into them while unloaded |
| Typing into a far village ISP router's console (`ispShows`) | router_ring_tap_* | The ISP router is hundreds of blocks away and usually headless. A player would walk there or ssh in. | Read-mostly: `router` / `show ...` / `exit` |
| Second scenario players (`Onlooker`, `Twin`) | phone_visibility, phone_duplicates | Multiplayer behaviour needs a second player, and a GameTest has none | Both act through the same game calls as the first |
| Dimension change by moving the fake player | phone_duplicates | A fake player can't take a portal | — |
| Handheld receiver run without a holder (`HandheldServer.Session.receive` at the listener) | radio_station | Handheld sessions tick per player, and the scenario player never ticks | It is the item's own server-side receiver |
| Probe apps copied into a phone's app folder; permissions set in the app manager | phone_gui, phone_background, phone_gba | No repository or package carries the `phonetest` probes (the phone's Terminal could install one with `pkg`, but there is none to install), and the phone has no settings page for these toggles | — |
| Compressed timers: the background refresh interval (minutes to 2-3 s) and the AP inactivity timeout (300 s to 5 s, GameTest hook) | phone_background, cell_chat; wifi_5ghz (GameTest only) | The real values don't fit a one-minute test, and the config's minimum for refresh is 1 minute | — |

Allowed without an entry, because each is a real action a player or operator takes:

- Operator commands with cheats on: `/forceload`, `/gamerule`, `/summon`, `/give`, and the diagnostic `/ecm radio link`.
- Server-config values an operator edits in `serverconfig`: `PhoneConfig.override`, and `EcmConfig.setUiClientRendering`, which writes the file.
- Vanilla terrain and filled chests set with setBlock, like `/fill` or a creative build.
- Creative inventory moves and hotbar selection.
- Packet handlers' server bodies run with the scenario player as sender: AP Configure, phone hotkeys, controller input, and the phone's display prefs (`PhoneServer.setPrefs`).
