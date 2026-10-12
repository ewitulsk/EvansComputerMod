# ePhone, as built (P5a: hardware)

The ePhone is a computer you carry. This page covers the hardware layer: the item, the
held-item host and its power states, slots, battery and Charging Dock, rendering in the hand,
visibility to other players, open mode and in-hand hotkeys. The phone shell, iPhone-style app
lifecycle and the GBA app come in a later lane; the seams they use are listed at the end.
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
- **In hand**: `PhoneClient` reads the seven mappings with `isDown`/`consumeClick` only and never
  touches another mapping, so binding slot 1 to W both walks and presses the slot (NeoForge sets
  every mapping on a key). Presses go out as `PhonePackets.Hotkey(phone, slot, down)` (slot 0 =
  switch app); the server takes them only from a player holding that phone, awake.

## Seams for the next lanes

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

- `phone_basics`: held = awake (boots, console 30x25); a Python counter runs; hotkey slot 1 down
  and up arrive as `phone_hotkey` events (control: refused with the phone only in the hotbar);
  hotbar = asleep, counter frozen for 1.5 s; back in hand it continues on the same computer; into
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
hand photographed showing the shell's green text (paired with the server's state), hotkey slot 1
bound to W while W is held: the player walks (6 blocks) and the server receives
`phone_hotkey(1, true/false)`; a docked phone in a lit Charging Dock with its charge bar.

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
