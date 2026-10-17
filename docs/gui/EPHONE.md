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

`PhoneServer` scans every player's inventory (hotbar, main inventory and off hand) each server tick
and tells each phone's `PhoneHost` where it is.

| Where | State | What happens |
| --- | --- | --- |
| Main hand (selected hotbar slot) or off hand | **AWAKE** | Host attached to the computer's `ComputerHost`, registered by UUID, modules live, `ComputerInstance.wake()`. First time: boots (`terminal_os`, no wired NICs). |
| Hotbar, not selected, or the main inventory (a pocket) | **ASLEEP** | `ComputerInstance.sleep()` (kernel worker and every child stop at their next safe point). Modules stay live (Wi-Fi stays associated; the AP may time it out). Refresh wakes, wake sockets and background modes keep running, so a call or music goes on in the pocket ([APPS.md](APPS.md#background-ephone)). |
| Anywhere else (chest, ground, dock) | **OFF** | Sleep, modules unloaded (radios leave the air), and once the sleep has landed (`awaitAsleep`, or 40 ticks) the host detaches: the computer stays in memory, paused. Picked up again, it resumes where it was. |
| Holder logs out, or the battery hits 0 | stopped | The computer is closed (`ComputerHost.forget()`); it cold-boots next time. Server stop does the same (`ComputerHost.shutdown`). |

Robustness: duplicates of one phone are one computer; when copies are in several places the
best placement wins (held over carried) and on a tie the current holder keeps it, so two players
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
  (`ModemModule` / `FramedModemModule.setSimProvider`, set before the module is attached): pulling the SIM
  drops the cell link at once. The tray is also the peripheral `sim` (`card/SimPeripheral`: `present`,
  `identity`, `respond(rand_hex)`; never the key), through which `celld` authenticates.
- **Cellular (v2).** While a modem module (a framed radio) and a written SIM are in, the phone runs
  `celld --phone` (`PhoneUi.cellular`); it reports bars and carrier with `/dev/uictl cell <bars> <carrier>`.
  See [CELLULAR_V2.md](CELLULAR_V2.md#as-built-phase-3-phone-side).
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
  The quad is drawn inside the display transform, so it turns with the body. The transforms
  (`models/ephone_body.bbmodel`, exported by `scripts/export-radio-models.py`) hold the glass towards
  the eye in first person (rotation -5, 155, 0: about 7 degrees off in either hand; Minecraft mirrors
  the left hand) and, in third person (25, 180, 0), up and back towards the holder's face.
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

- **Input priority**: `PhoneScreen` and the Terminal's screen are `ComputerInputScreen`s:
  `client/ComputerScreenInput` takes the mouse over them (the whole phone screen; the Terminal's
  panel and controller toggle) and every key at the highest priority of NeoForge's screen events,
  hands it to the screen and cancels the event, so overlays such as JEI's (which act on the same
  events, also while hidden) can't eat a click on a window's close button. The Terminal's screen also
  reports its real size (`imageWidth`/`imageHeight`), so JEI places its overlay beside it.
- **Open** (`PhoneScreen`): keys become `TerminalInputPacket`s (typed text; VT sequences for
  arrows, Home/End, PgUp/PgDn, Delete, Enter, Backspace, Tab; Ctrl+letter as control
  characters); the mouse is a finger: `MouseInputPacket` down/up/move/scroll on display 0 in
  phone pixels (graphics pixels, letterbox removed, while a program shows graphics). Both are
  addressed by the phone's UUID; the server accepts them only from the holder of an awake phone
  (`IComputerHost.acceptsInput`, checked by `ComputerTargets.resolveInput`). Pointer events reach
  programs through `mouse_poll` once they capture the mouse. Escape closes; the screen closes
  itself if the phone leaves the hand.
- **In hand** (P5b: slots go to the open app; a slot the open app hasn't claimed, or any slot at home, goes to the app with the active media session if it claimed it (`Compositor.activeMedia`: Music's Play/Pause, Previous, Next from home and in the background); the switch-app key to the shell; see
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
kernel's console keeps running behind the GUI. Players get a shell through the built-in
**Terminal** app (below), which the phone scenarios type into.
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
and cellular bars, the carrier celld is on (`carrier <name|->`, drawn before the bars), the foreground
app, active background modes, hotkey claims). The shell sends the
`ecm-phone` theme (the default theme with its green accent). Safe mode: a restarted shell
(`--restart`) gets `WindowOpened` for the open apps and doesn't reopen the last session again.

### Apps on a phone

- **Built-in apps** are installed into `/apps` at every boot when missing or changed
  (`PhoneUi.provision`, from `assets/evanscomputermod/ecm_apps/<id>/` and the server's programs):
  **Notes** (`dev.ecm.notes`, the `notes` program; slot 1 = Add note), **GBA** (`dev.ecm.gba`,
  the `gba-app` program) and **Terminal** (`dev.ecm.terminal`, the desktop's `terminal` program:
  a kernel shell session in a window, no permissions; the programs typed in it are the kernel's,
  unconfined, as on a desktop). They are ordinary apps (`.pkg-origin` = `builtin`). The system apps
  every computer has are on the grid too, among them the **Video Player** (`dev.ecm.video`, hotkey
  slots 1 play/pause, 2 back 10 s, 3 forward 10 s; [APPS.md](APPS.md#video-player)).
- **Terminal on a phone**: the window between the bars is 240 lu wide, and below 320 lu the
  Terminal drops its 2 lu padding, so the grid is 30x22 (the phone console's 30 columns). There
  is no on-screen keyboard: the open phone has the player's keyboard (typed text, Enter, Ctrl+C /
  Ctrl+T stop the program).
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
| The phone leaves the hand (hotbar, pocket or away) | `PhoneLifecycle.willSleep` -> `PhoneUi.willSleep` suspends every running app on a worker and holds the computer's sleep for their save deadline plus 0.3 s (at most 2 s). |
| Back in hand | `PhoneUi.wake`: the foreground app resumes. |
| In the hotbar or the pocket | Refresh wakes from the app manager's `BackgroundScheduler` (its own 1 s timer): an app that declared `background = { refresh = true }` and holds `background:refresh` is resumed for `apps.refreshBudgetMs` (2 s) every `apps.refreshIntervalMinutes` (5), the computer woken for the window (`wakeFor`), then paused again; `BackgroundDone` ends it early. |
| Background modes | An app starts `audio`, `call`, `radio`, `transfer` or `navigation` with `BackgroundMode{mode, on}`; `audio` also starts by itself when an app that declared it writes to the phone's speaker (`SpeakerAudio` write listener -> `PhoneUi.audioActivity`) and stops 2 s after its last write. The first time the app is hidden with a mode on, the shell asks once ("Keep Discord running in the background for calls?"). While on and allowed, the suspended app runs duty-cycled (`apps.backgroundCpuPercent.<mode>`: 10 % of every 100 ms, 25 % for a call, waking the sleeping computer for each slice); the status bar shows `AUDIO` / `CALL`, and `MIC` while a hidden app reads the microphone. At most `apps.maxBackgroundApps` (2) apps at once; the oldest is stopped and a notification names it. |
| Wake sockets | Data on a socket with `SO_ECM_WAKE` gives its suspended app a refresh window at once (a sleeping phone lets network interrupts run the kernel briefly to see it): a Discord message notifies in seconds. |
| Now Playing | `MediaSession` from an app -> a card in the notification sheet (art, title, progress, previous / play-pause / next); a double-tap on worn AirPods plays or pauses it. The sheet's volume slider expands into the Volume Mixer's strips. |

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

- **GAMES** (the pill between L and R while a game runs) goes back to the game list. The
  cartridge save is written first, so the game picks up where its own save left it.
- **Settings** (top right of the game list) is the Controls page: one row per GBA button (A, B,
  L, R, Start, Select, Up, Down, Left, Right) with its keyboard key. Tap a row, then press the
  key; Esc (or tapping the row again) cancels. A key another button already uses is refused and
  the message names that button ("Space is already Start..."); change that one first.
  **Reset to defaults** puts back the keys below. Every change is saved at once in
  `/var/apps/dev.ecm.gba/keys.json` (`{"version":1,"keys":{"A":29,...}}`, HID usages; a file
  that binds one key twice is ignored). The rows fill as many columns as the window's height
  needs, so the page fits the phone and a short Terminal window alike.
- The screens and the bindings are `rust/wasm-programs/gba-app/src/lib.rs` (tested on the host:
  `cargo test -p gba-app`); `main.rs` is the emulator and the connection. The in-hand hotkey
  slots below are separate and don't change.

| Input | Mapping |
| --- | --- |
| In hand (hotkey slots) | 1 = A, 2 = B, 3 = Left, 4 = Right, 5 = Up, 6 = Start (Down, L, R and Select need the phone open or a controller: six slots) |
| Open: keyboard | by default arrows = D-pad, Z = A, X = B, A = L, S = R, Enter = Start, Backspace = Select; rebound on the Settings page |
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
  hotkey claims, slot 1 to the game; GAMES back to the list, Settings: Start rebound to Space
  (in `keys.json`), A refused Space (the conflict named).
- Golden images `phone_home`, `phone_sheet`, `phone_dialog` (`UiGoldenTest`): recorded from the
  shell's own policy with `cargo run -p phone --example record_phone_sessions -- ../src/test/resources/ui-sessions`.
- Golden images `gba_games`, `gba_settings` (240x360) and `gba_playing` (640x400, the test ROM
  with GAMES between L and R): `cargo run -p gba-app --example record_gba_sessions -- ../src/test/resources/ui-sessions`.
- Scenarios (and `ecm_phone` GameTests): `phone_gui`, `phone_background`, `phone_gba` (it also
  taps GAMES, opens Settings, rebinds Start to Space and checks the control conflict, then plays
  again)
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

`/ecm scenario spawn phone_basics|phone_battery|phone_dock|phone_visibility|phone_duplicates|phone_wifi_walk`
(auto, manual, fast; `/ecm scenario commands <name>`). In a normal world the holder is the
scenario's unseen builder player, so follow the chat; the commands text says how to do it with
your own phone. The same definitions are the GameTests (`-GameTests ecm_phone`, structure
`gametest_phone`):

- `phone_basics`: held = awake (boots, console 30x25, the phone shell up); a Python counter runs on
  the console behind the GUI; hotkey slot 1 is taken from the holder (it goes to the open app,
  `phone_gui` checks that; control: refused with the phone only in the hotbar); hotbar = asleep, counter frozen for 1.5 s; back in hand it continues on the same computer; in
  the main inventory = asleep and still attached (a pocket); into a chest (placed, opened, the phone
  shift-clicked in) = off (detached, same computer paused, frozen); taken back and held it resumes
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
- `phone_dock` (real time): a half-charged phone docked (the dock holds it, lamp lit, phone off)
  and taken back with an empty hand (dock empty, lamp off, phone asleep in the hotbar), twice; each
  time the dock's update packet and update tag are applied through the client's own path
  (`onDataPacket`, `handleUpdateTag`) to a client-side copy of the dock, which must follow
  (control: the empty update tag sent before the fix leaves the phone on a client's dock).
- `phone_wifi_walk` (real time): a gateway computer cabled to an open AP at 0 dBm; the phone runs
  `wifi connect ecm-walk` (the AP lists it), gets an address by hand and pings the gateway; its
  holder walks 28 blocks away (0.2 blocks a tick, the module pose following); the AP still lists
  it and pings are answered; `/ecm radio link` puts the open spot over the module's -92 dBm
  sensitivity (about -65 dBm) and the spot behind a 4-block stone wall far under it; there, pings
  get no answer.

Client checks (`-ClientChecks -ClientSuite phone`): the phone in the main hand and then the off
hand photographed showing the phone shell (paired with the server's state; the glass must face the
eye within 25 degrees), in third person from behind (the glass within 60 degrees of the holder's
eyes), opened with a click on
Notes, hotkey slot 1 bound to W while W is held: the player walks (6 blocks) and the open app gets
the slot; a docked phone in a lit Charging Dock with its charge bar, then taken off with the use key and an
empty hand: the client's dock is empty, its lamp off, no charge bar (P5b details above).

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
  (`LowMac.bssSignalDbm`: >= -55 dBm 4 bars ... < -85 dBm 0), cellular bars from celld's report.
- P5b: notifications aren't permission-checked yet (`notifications` is declared by apps but the
  compositor posts every `Notify`); the general app wiring step owns that.
- P5b: only `audio` starts by itself; `radio`, `transfer` and `navigation` have no producer on a phone
  yet. Duty cycling and refresh budgets are wall-clock (APPS.md); a pure compute loop on Wasmtime only
  stops at its next host call.
- P5b: the GBA restarts its game after a cold boot (the emulator's save states are lossy), keeping
  the cartridge save. Down, L, R and Select are not on the six in-hand slots.
- P5b: apps reopened at a cold boot come back in the background (home screen shown), not on screen.
- P5b: a phone's App Store comes from another lane. Settings is built in since Cellular v2 phase 3
  (`dev.ecm.settings`: Wi-Fi, Cellular, Display, Appearance, Apps, About; its permissions allowed once at install);
  background modes are still granted with `pkg perm` (never prompted, as APPS.md says).
