# ePhone, as built (P5)

The ePhone is a computer you carry. This page covers the hardware layer (P5a: the item, the
held-item host and its power states, slots, battery and Charging Dock, rendering in the hand,
visibility to other players, open mode and in-hand hotkeys) and the phone GUI (P5b: always in GUI
mode with the phone shell, apps one at a time with iPhone-style lifecycle, hotkeys to the open app,
the GBA app): see [The phone GUI](#the-phone-gui-p5b).
Decisions it follows: [DISPLAY_PIPELINE §7](DISPLAY_PIPELINE.md#7--ephone-decisions),
[UI_PROTOCOL §4, §7, §11, §15](UI_PROTOCOL.md). Code: `phone/` (1.21.1 only, like the radio
and cellular code it builds on).

## What players get

| Thing | How |
| --- | --- |
| **ePhone** (`evanscomputermod:ephone`) | Crafted from 6 iron, a glass pane, a Color LCD panel and a Wi-Fi Module. Comes with a Color LCD panel and a Wi-Fi Module fitted, 100 % charge. |
| Use it | Hold it (either hand): it runs. Right-click: open it full screen (every key and the mouse go to the phone; Escape closes). Sneak + right-click: its slot screen. |
| **Slot screen** | Display panel (Tier 1-3; portrait 120x200 / 240x400 / 480x800), two module slots, SIM tray, plus two toggles: *Share screen* and *Others' phones* (saved in `config/evanscomputermod-phone-client.toml`). |
| **Hotkeys** | Controls > ePhone: *ePhone slot 1-6* and *ePhone: switch app*, all unbound. Active while a phone is in the main or off hand and no screen is open. |
| **Charging Dock** (`evanscomputermod:charging_dock`) | 2 gold nuggets, redstone, 3 iron. Right-click it with a phone to dock it, with an empty hand to take it back. Its lamp is lit while it charges; it also accepts FE from any side. |

## States

`PhoneServer` scans every player's hotbar and off hand each server tick and tells each phone's
`PhoneHost` where it is.

| Where | State | What happens |
| --- | --- | --- |
| Main hand (selected hotbar slot) or off hand | **AWAKE** | Host attached to the computer's `ComputerHost`, registered by UUID, modules live, `ComputerInstance.wake()`. First time: boots (`terminal_os`, no wired NICs). |
| Hotbar, not selected | **ASLEEP** | `ComputerInstance.sleep()` (kernel worker and every child stop at their next safe point). Modules stay live (Wi-Fi stays associated; the AP may time it out). |
| Anywhere else (inventory, chest, ground, dock) | **OFF** | Sleep, modules unloaded (radios leave the air), and once the sleep has landed (`awaitAsleep`, or 40 ticks) the host detaches: the computer stays in memory, paused. Picked up again, it resumes where it was. |
| Holder logs out, or the battery hits 0 | stopped | The computer is closed (`ComputerHost.forget()`); it cold-boots next time. Server stop does the same (`ComputerHost.shutdown`). |

Robustness: duplicates of one phone are one computer; when copies are in several places the
best placement wins (held over hotbar) and on a tie the current holder keeps it, so two players
holding copies don't make it flap. Swapping hands keeps it awake; dropping it, death (items drop)
or putting it in a chest turns it off; a dimension change moves its pose (and radios) with the
player. The FakePlayers of scenarios are included with `PhoneServer.track`.

Documented limits: the computer doesn't survive a logout or restart (no snapshot of a running
computer); app state survives through the app platform's save/restore (P4). A holder who logs
out with a copy also held by someone else stops it; the other copy cold-boots it. Only the
holder (the one the scan picked) gets display frames; a second player holding a copy sees the
last picture their client had.

## Hardware and data

Stack components: `evanscomputermod:phone_id` (UUID, given on first use; a phone that has never
been used has no computer), `phone_battery` (hundredths of a percent) and `phone_hardware`
(panel, module 1, module 2, SIM; a stack without it has the factory fit). Files live in
`<world>/computer-data/<uuid>` like any computer.

- **Panel.** The console follows the panel's portrait text grid (`PanelTier.modes().phone()`:
  15x12, 30x25, 30x25 at scale 2) through `ComputerInstance.requestConsoleResize`. No panel =
  headless. The host reports `portraitDisplay()`, so program framebuffer limits and
  `gfx_modes`' text grid and largest size come from the portrait mode.
- **Modules.** A `ModuleBays` with one bay open; programs see `module_1` and `module_2`
  (`ModuleBays.Owner.slotName`). The module pose is the holder's eye (0.3 lower), updated every
  tick, so the Wi-Fi module and modems move with the player (the handheld radio's pattern).
- **SIM tray.** Modems in the phone get `PhoneHost::sim` as their `SimProvider`
  (`ModemModule.setSimProvider`): pulling the SIM drops the cell link at once.
- The slot screen writes changes to the stack and applies them live (`PhoneHost.setHardware`).

## Battery

Drained per server tick while the host is attached, written to the stack (at most once a
second, so a dropped phone loses at most a second). Per minute: awake `awakePerMinute` times the
panel factor, asleep `asleepPerMinute`, plus `radioPerMinute` per active radio module (Wi-Fi on,
modem attached). At 0 the phone powers off (the holder is told) and stays off in the hand until
charged. A docked phone charges at `dockPerMinute`; with `dockFePerPercent` above 0 the dock only
charges from the FE it stores (`RadioEnergyStorage`, 100 kFE, 1 kFE/t in). The item shows a
charge bar under 100 %.

**Programs** read it from the `phone` peripheral (type `ephone`):

```python
import peripheral
p = peripheral.find("ephone")
print(p.battery(), p.state())      # 87.5 awake
print(p.info())                    # battery, state, tier, width, height, cols, rows, scale, holder, sim
```

## Rendering

- `PhoneItemRenderer` (the item's custom renderer: `models/item/ephone.json` is
  `builtin/entity` with the Blockbench display transforms) draws the Blockbench body
  (`models/item/ephone_body.json`) and a quad over the glass (x 5-11, y 2.5-12.5 px, north face).
- `PhoneScreenTexture` composes the phone's display from the client display cache
  (UUID + display id) into one texture at the panel's portrait resolution: the graphics plane
  (letterboxed), the text grid in the terminal font (8x16 cells times the scale, colours,
  bold/underline/inverse/blink, cursor), Tier 1 in two phosphor tones. Recomposed only when the
  picture changes. The open-phone screen and the dock use the same texture.
- Your own phone (first person, inventory, or any phone in your inventory) shows display 0 while
  awake and is dark asleep or off. A docked phone shows dark glass with a charge bar.
- Frames: display 0 is streamed to the holder only (`DisplaySyncHub`, one delta per frame). The
  client asks for a keyframe (`TerminalReadyPacket`, generation 0) when it has no picture.

## Visibility to others

Three settings: the server's `visibility.max` (`OFF` / `GLOW` / `FULL`), the holder's *Share
screen* (off lowers FULL to GLOW), and each onlooker's *Others' phones* (off: they render every
other phone dark and the server sends them nothing). `PhonePackets.Status` (state, visibility,
battery) goes to players within 48 blocks once a second and on change.

- **FULL**: display `PhoneHost.DISPLAY_MIRROR` (2), the same picture, is streamed to players
  within `mirrorRange` who render others, at most every `mirrorIntervalTicks`. Onlookers may ack
  it (`canInteract`) but never send input (`acceptsInput` is the holder's, display 0, awake).
- **GLOW**: a lit, plain screen; no display packets at all.
- **OFF** (or not awake): dark.

## Open mode and hotkeys

- **Open** (`PhoneScreen`): keys become `TerminalInputPacket`s (typed text; VT sequences for
  arrows, Home/End, PgUp/PgDn, Delete, Enter, Backspace, Tab; Ctrl+letter as control
  characters); the mouse is a finger: `MouseInputPacket` down/up/move/scroll on display 0 in
  phone pixels (graphics pixels, letterbox removed, while a program shows graphics). Both are
  addressed by the phone's UUID; the server accepts them only from the holder of an awake phone
  (`IComputerHost.acceptsInput`, checked by `ComputerTargets.resolveInput`). Pointer events reach
  programs through `mouse_poll` once they capture the mouse. Escape closes; the screen closes
  itself if the phone leaves the hand.
- **In hand** (P5b: slots go to the open app, the switch-app key to the shell; see
  [Input](#input)): `PhoneClient` reads the seven mappings with `isDown`/`consumeClick` only and never
  touches another mapping, so binding slot 1 to W both walks and presses the slot (NeoForge sets
  every mapping on a key). Presses go out as `PhonePackets.Hotkey(phone, slot, down)` (slot 0 =
  switch app); the server takes them only from a player holding that phone, awake.

## The phone GUI (P5b)

A phone is always in GUI mode. When its computer boots, `UiManager` sees a portrait host
(`ComputerInstance.isPhone()`) and creates a `PhoneUi` (`computer/ui/PhoneUi.java`,
version-neutral); the compositor starts at once on the panel's portrait display
(`UiDisplay.forTier(tier, EPHONE)`: 120x200 / 240x400 / 240x400 at scale 2) with the phone shell
`dev.ecm.phone` pinned (`/etc/ui.toml`'s `shell` is not used on a phone; its theme and
`shell_args` are). There is no console screen: Ctrl+Alt+Backspace does nothing on a phone. The
kernel's console keeps running behind the GUI (the P5a scenarios drive it with
`ComputerInstance.consoleInput`); players will reach it through the Terminal app (P3).
A panel swap in the slot screen sends the shell `DisplayChanged` (`PhoneHost.applyPanel` ->
`UiManager.panelChanged`), and it lays its layers out again at the new size.

### The phone shell `dev.ecm.phone`

Program `phone` (`rust/wasm-programs/phone`: the policy is the library `phone_shell`, the program
reads the computer). It owns only layer surfaces, never windows:

| Layer | What |
| --- | --- |
| `home` (background) | Wallpaper and the grid of installed apps (`app_ctl list`, each app's `icon.png`; apps without one get a coloured tile with an initial), plus the shell's own **Controls** entry. 4 columns at 240 lu, 3 at 120. |
| `status` (top, 16 lu) | Clock, a dot for unread notifications, a pill per background mode in use (`AUDIO`, `RADIO`, `DL`, `NAV`), cellular bars, Wi-Fi fan, battery % and gauge. Tap or pull it down: the notification sheet. |
| `nav` (top, 20 lu at the bottom) | Back, Home, Recent apps. |
| `overlay` (made while shown) | The notification sheet (`NotificationPosted`; tap = `NotificationInvoke` and open the app; Clear all), the app switcher (recent apps; tap to show, X to close), the permission dialog (`PermissionPrompt` -> Allow / Only this time / Don't allow = `AnswerPermission{allow, remember}`), the Controls screen (every open app's hotkey labels, the open one first, and the keys below). |

One app is on screen at a time, placed between the bars (`MAXIMIZED | ACTIVATED`); every other app
window is hidden (`SetVisible false`), which also takes their focus, so the OS suspends them.
Keys (the shell has no window, so they are chords): **Alt+H** home, **Alt+Backspace** back (closes
an overlay, then goes home), **Alt+Tab** next open app, **Alt+N** notification sheet. Plain
Backspace and Home stay with the app (text fields need them); Escape closes the phone on the client.
The status comes from `/dev/uictl phone` once a second (`PhoneUi.statusText`: state, battery, Wi-Fi
and cellular bars, the foreground app, active background modes, hotkey claims). The shell sends the
`ecm-phone` theme (the default theme with its green accent). Safe mode: a restarted shell
(`--restart`) gets `WindowOpened` for the open apps and doesn't reopen the last session again.

### Apps on a phone

- **Built-in apps** are installed into `/apps` at every boot when missing or changed
  (`PhoneUi.provision`, from `assets/evanscomputermod/ecm_apps/<id>/` and the server's programs):
  **Notes** (`dev.ecm.notes`, the `notes` program; slot 1 = Add note) and **GBA** (`dev.ecm.gba`,
  the `gba-app` program). They are ordinary apps (`.pkg-origin` = `builtin`).
- **Launch** of an installed app goes through the app manager (`PhoneUi.launch`, one instance per
  app), so apps have their identity, permissions and lifecycle; anything else falls back to a plain
  program as before. The session (open apps) is written on each launch and at shutdown.
- **Cold boot** (logout, empty battery, server restart): the shell relaunches the last session's
  apps in the background (in the switcher, the home screen shown) and each gets `Restore` with the
  state it saved.

### Lifecycle (iPhone rules, `LifecyclePolicy.PHONE`)

| When | What happens |
| --- | --- |
| An app's window gets focus (the shell shows it) | It is the foreground app; if suspended, `resume` (its process group runs again). |
| Focus leaves it (home, another app) | `suspend`: `WillSuspend{500 ms}` over its UI connection -> `SaveState` kept in `/var/apps/<id>/state` -> its process group paused. Apps started in the last 5 s are left alone while they open their window; a sweep every 0.5 s suspends any other background app. |
| The phone leaves the hand (hotbar or away) | `PhoneLifecycle.willSleep` -> `PhoneUi.willSleep` suspends every running app on a worker and holds the computer's sleep for their save deadline plus 0.3 s (at most 2 s). |
| Back in hand | `PhoneUi.wake`: the foreground app resumes. |
| In the hotbar | Refresh wakes from the app manager's `BackgroundScheduler` (its own 1 s timer): an app that declared `background = { refresh = true }` and holds `background:refresh` is resumed for `apps.refreshBudgetMs` (2 s) every `apps.refreshIntervalMinutes` (5), the computer woken for the window (`wakeFor`), then paused again; `BackgroundDone` ends it early. |
| Background modes | `audio` starts by itself when an app that declared it (and holds `background:audio`) writes to the phone's speaker (`SpeakerAudio` write listener -> `PhoneUi.audioActivity`) and stops 2 s after its last write. While on, the suspended app runs duty-cycled (10 % of every 100 ms, waking the sleeping computer for each slice); the status bar shows `AUDIO`. The other modes are started by `BackgroundScheduler.startMode` as before (nothing calls them on a phone yet). |

### Input

- **Hotkey slots** (in hand): `PhoneApps` installs the `PhoneHotkeys` sink: slot 1-6 ->
  `Compositor.hotkey` -> `Hotkey{slot, down}` to the app whose window has focus, if that app claimed
  the slot (`HotkeyClaim{slot, label}`, labels in the Controls screen); unclaimed slots and slots
  with nothing open go nowhere. The **switch-app key** -> `Chord{0xEC01}` to the shell
  (`Compositor.CHORD_SWITCH_APP`), which shows the next open app without the phone being opened
  (safe mode: the next window gets focus). A phone whose GUI isn't up yet falls back to the P5a
  peripheral events.
- **Open mode**: `PhoneScreen` sends every key as a raw HID key (`KeyInputPacket`, down / repeat /
  up) and typed text, next to the console strings; the server uses whichever matches what is shown.
  The mouse is the finger (`MouseInputPacket` on display 0 -> the compositor's input router). Key
  packets now go through `ComputerTargets.resolveInput` like the others (only the holder of an awake
  phone types).

### Speaker, controller, client rendering

- **Speaker.** Every phone has one: peripheral `speaker` (`/dev/audio`, `/dev/audioctl`), heard
  within `speakerRange` of the holder (`SpeakerAudioPacket` at the holder's position). Apps need
  `peripherals:speaker`.
- **Wireless Controller.** Right-click a controller with the phone in the other hand: it pairs with
  the phone (`ControllerServer.pairPhone`). The phone's own receiver connects it as `controller_1..4`
  while the phone is attached (no receiver module, no radio link: it is in your hands).
- **Client rendering.** A holder whose client draws scenes (UI_PROTOCOL §17) gets the phone's GUI as
  the scene stream (`PhoneHost.sceneSync`): `ClientUiScenes` fills the same display cache entry the
  in-hand renderer and the open screen read. Others, and the mirror for onlookers (display 2), stay
  on tiles.

### GBA

`gba-app` (`dev.ecm.gba`): one window with the game in a `canvas` node (frames go in with
`ui_put_pixels`, RGB565, half size under 240 lu) and touch buttons around it. Without a ROM it lists
the `.gba` files in `/home` and `/home/roms` (`files:documents`). It saves which ROM runs and
reopens it after a cold boot; the game's own save goes next to the ROM (`.sav`).

| Input | Mapping |
| --- | --- |
| In hand (hotkey slots) | 1 = A, 2 = B, 3 = Left, 4 = Right, 5 = Up, 6 = Start (Down, L, R and Select need the phone open or a controller: six slots) |
| Open: keyboard | arrows = D-pad, Z = A, X = B, A = L, S = R, Enter = Start, Backspace = Select |
| Open: touch | the on-screen D-pad, A, B, L, R, Start, Select |
| Controller | A/B, LB/RB = L/R, Start, Back = Select, D-pad or left stick (needs `controller`) |

### Tests and scenarios (P5b)

- `PhoneKernelTest` (JUnit 26.1, real kernel on Chicory, a portrait test host): boots into the
  shell (EPHONE 240x400, Notes and GBA on the grid, no console switch); Notes opens from the grid
  between the bars as an app instance, typed text, slot 1 adds the note, an unclaimed slot goes
  nowhere, Alt+H suspends it after SaveState, the switch-app key brings it back, a second app
  suspends Notes; a Tier 1 panel hot-plug relays it out at 120x200. Sleep and wake keep a counter's
  state (frozen asleep, same process, no catch-up) and a cold boot reopens it with `Restore`. A
  refresh wake while asleep posts a notification and is cut by its 1 s budget (control: nothing
  without `background:refresh`). Audio: the permission dialog (tapped), then without
  `background:audio` a suspended app is silent and with it it keeps playing asleep (about 24,000 of
  24,000 samples in 3 s). The GBA app: the documents dialog, the ROM list, the game drawing, its six
  hotkey claims, slot 1 to the game.
- Golden images `phone_home`, `phone_sheet`, `phone_dialog` (`UiGoldenTest`): recorded from the
  shell's own policy with `cargo run -p phone --example record_phone_sessions -- ../src/test/resources/ui-sessions`.
- Scenarios (and `ecm_phone` GameTests): `phone_gui`, `phone_background`, `phone_gba`
  (`testing/scenario/PhoneGuiScenarios.java`; `/ecm scenario spawn phone_gui` etc., auto/manual/fast
  like the others; batteries are off for these runs because a GameTest server ticks unthrottled).
- Client suite `phone`: the shell in the main and off hand (drawn from the scene stream, its
  wallpaper counted in the photo), the phone opened and Notes clicked (the server sees Notes on
  screen), slot 1 on W in the off hand: the player walks and the open Counter app gets "Bump".

## Seams (P5a), and how P5b uses them

P5b installs both (`phone/PhoneApps`, at mod setup): the hotkey sink routes to the compositor and
the lifecycle listener suspends and resumes apps.

- **Compositor** (P2): `PhoneHotkeys.setFactory(host -> sink)` installs a `PhoneHotkeySink`
  (`slot(phone, slot, down)`, `switchApp(phone)`), looked up on every press. Until then the
  default posts peripheral events `phone_hotkey(slot, down)` and `phone_switch_app()` from the
  `phone` peripheral (`peripheral.pull_event("phone_hotkey")`). Open-mode input arrives through
  the ordinary UUID input packets on display 0; the phone's display is portrait
  (`portraitDisplay()`, `displayTier(0)`), matching `UiDisplay.forTier(tier, EPHONE)`.
- **App platform** (P4): `PhoneLifecycle.addListener(...)`: `willSleep(phone, next)` may hold a
  sleep back up to 2 s (deliver `WillSuspend`), `stateChanged(phone, from, to)` (e.g. set
  `LifecyclePolicy.PHONE` on `getAppManager()` and suspend/resume the foreground app),
  `stopped(phone, why)`. While a phone is ASLEEP the background scheduler gets refresh windows
  with `PhoneHost.refreshWake(budgetMs)` (a `ComputerInstance.wakeFor`; null unless asleep and
  booted). `PhoneServer.find(server, uuid)` / `hosts(server)` find hosts.

## Configuration

`<world>/serverconfig/evanscomputermod-phone-server.toml`:

| Key | Default | |
| --- | --- | --- |
| `visibility.max` | `FULL` | `OFF`, `GLOW` or `FULL` |
| `visibility.mirrorRange` | 16 | blocks |
| `visibility.mirrorIntervalTicks` | 10 | onlookers' refresh |
| `battery.enabled` | true | false: phones never run down |
| `battery.awakePerMinute` | 1.0 | % per minute held, Tier 2 |
| `battery.asleepPerMinute` | 0.1 | in the hotbar |
| `battery.offPerMinute` | 0.0 | |
| `battery.radioPerMinute` | 0.3 | per active radio module |
| `battery.tier1Factor` / `tier2Factor` / `tier3Factor` | 0.7 / 1.0 / 1.5 | awake drain by panel |
| `battery.dockPerMinute` | 20.0 | charge rate |
| `battery.dockFePerPercent` | 0 | FE per % (0 = free) |
| `battery.newPhonePercent` | 100 | |

Client (`config/evanscomputermod-phone-client.toml`): `shareScreen`, `renderOthers` (both true).

## Tests and scenarios

`/ecm scenario spawn phone_basics|phone_battery|phone_visibility|phone_duplicates|phone_wifi_walk`
(auto, manual, fast; `/ecm scenario commands <name>`). In a normal world the holder is the
scenario's unseen builder player, so follow the chat; the commands text says how to do it with
your own phone. The same definitions are the GameTests (`-GameTests ecm_phone`, structure
`gametest_phone`):

- `phone_basics`: held = awake (boots, console 30x25, the phone shell up); a Python counter runs on
  the console behind the GUI; hotkey slot 1 is taken from the holder (it goes to the open app,
  `phone_gui` checks that; control: refused with the phone only in the hotbar); hotbar = asleep, counter frozen for 1.5 s; back in hand it continues on the same computer; into
  the inventory = off (detached, same computer paused, frozen); back in the hotbar it resumes
  where it was; off hand = awake.
- `phone_battery` (real time): drains while held (written to the item); docked = off, lamp lit,
  charging; an almost flat battery powers it off and stops the computer (control: it stays off in
  hand); docked again it charges and then cold-boots.
- `phone_visibility`: GLOW sends a nearby onlooker zero display packets over 2 s; FULL puts them
  in the mirror and sends packets (control); holder's share off = GLOW; onlooker's render-others
  off = out of the mirror.
- `phone_duplicates`: a second player holds a copy: no holder change or state change for 1.5 s;
  the first puts theirs away and the second takes over (same computer); the second goes to the
  Nether (pose follows); dropping it = off.
- `phone_wifi_walk` (real time): a gateway computer cabled to an open AP at 0 dBm; the phone runs
  `wifi connect ecm-walk` (the AP lists it), gets an address by hand and pings the gateway; its
  holder walks 28 blocks away (0.2 blocks a tick, the module pose following); the AP still lists
  it and pings are answered; `/ecm radio link` puts the open spot over the module's -92 dBm
  sensitivity (about -65 dBm) and the spot behind a 4-block stone wall far under it; there, pings
  get no answer.

Client checks (`-ClientChecks -ClientSuite phone`): the phone in the main hand and then the off
hand photographed showing the phone shell (paired with the server's state), opened with a click on
Notes, hotkey slot 1 bound to W while W is held: the player walks (6 blocks) and the open app gets
the slot; a docked phone in a lit Charging Dock with its charge bar (P5b details above).

## Deviations and gaps

- All phone code is 1.21.1-only (`//? if <=1.21.1`), like the radio and cellular code it uses.
- "Expected range" in the walk test is set by a wall, not free space: at 0 dBm the free-space
  range of the Wi-Fi module is about 390 blocks, too far for a one-minute test.
- The mirror and status reach onlookers in the same level only.
- Logout (the computer stops) is not covered by a test: scenario players can't log out.
- No `/dev/battery` file: the battery is on the `phone` peripheral.
- Off phones stay paused in memory for as long as the server runs (no idle timeout yet).
- The second holder of a duplicate sees a stale picture (frames go to the scanned holder only).
- The in-hand screen is drawn opaque (cutout render type with full-bright light); a translucent
  emissive quad never showed on items.
- P5b: the phone shell reads the battery and signal from `/dev/uictl phone` (fed by the phone
  host), not from the `phone` peripheral; Wi-Fi bars come from the joined AP's last frames
  (`LowMac.bssSignalDbm`: >= -55 dBm 4 bars ... < -85 dBm 0), cellular bars from the modem.
- P5b: notifications aren't permission-checked yet (`notifications` is declared by apps but the
  compositor posts every `Notify`); the general app wiring step owns that.
- P5b: only `audio` starts by itself; `radio`, `transfer` and `navigation` have no producer on a phone
  yet. Duty cycling and refresh budgets are wall-clock (APPS.md); a pure compute loop on Wasmtime only
  stops at its next host call.
- P5b: the GBA restarts its game after a cold boot (the emulator's save states are lossy), keeping
  the cartridge save. Down, L, R and Select are not on the six in-hand slots.
- P5b: apps reopened at a cold boot come back in the background (home screen shown), not on screen.
- P5b: a phone's Settings / Terminal / App Store come from the other lanes; until then permissions
  are reviewed with `pkg perm` and background modes are granted there (never prompted, as APPS.md says).
