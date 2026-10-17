# ComputerCraft programs on ECM computers

ComputerCraft: Tweaked (CC) programs run in our Lua unchanged. CC's globals are
always on, with no flag. Each one is a thin layer over an ECM module that Python
has too, and the logic lives in Rust (`rust/crates/ecm-console`).

Everything here is written from CC's public API docs (tweaked.cc). No CC ROM or
CC source is copied.

## Phase 1: the console (done)

| CC | Built on (Lua and Python) | Notes |
|---|---|---|
| `term.*` (camelCase and British spellings), `term.redirect` / `current` / `native`, `nativePaletteColour` | `term` (`term.write`, `term.set_cursor_pos`, ... in both) | 1-based cursor. `write` clips at the edges and doesn't wrap. `blit` errors when its lengths differ. A bad colour is an error. |
| `colors`, `colours` (`grey`, `lightGrey`) | `colors` | CC values (white = 1 ... black = 32768). `combine`, `subtract`, `test`, `packRGB`, `unpackRGB`, `toBlit`, `fromBlit`, `rgb8`. |
| `keys` | `keys` | GLFW key codes, as CC uses them. `keys.getName`. Python uses `keys.get("end")`. |
| `os.pullEvent`, `os.pullEventRaw`, `os.queueEvent`, `os.startTimer`, `os.cancelTimer`, `os.setAlarm`, `os.cancelAlarm`, `sleep` | `events` (`pull`, `queue`, `start_timer`, ...) | One queue per program: `key` / `key_up` / `char`, `timer`, `alarm`, peripheral events, and your own events. A filter discards the other events, but `terminate` always comes through. `peripheral.pull_event` reads the same queue. |
| `os.time`, `os.day`, `os.epoch`, `os.clock`, `os.getComputerID`, `os.computerID`, `os.getComputerLabel`, `os.setComputerLabel` | `computer` (`time`, `day`, `epoch`, `uptime`, `id`, `label`, `set_label`) | See the time notes below. |
| `write`, `print`, `printError`, `read` | `term.write_wrapped` | Word-wrapped through the term. `print` and `write` return the number of lines. `read(replace, history, complete, default)` edits the line and keeps history. |
| `os.version()`, `_HOST` | | "CraftOS 1.9", so version checks in CC programs pass. |

### Where CC and standard Lua differ, CC wins (Evan, 2026-10-17)

- **`os.time()`** is the in-game hour. With `"utc"` it is the real hour; with a table it gives Unix seconds as before.
- **`os.clock()`** is the program's uptime, not CPU time.
- **`print` and `io.write`** go through the term, so they wrap and use the term's colours. `io.stdout:write` still writes raw bytes, escape sequences included.
- **Python keeps standard behaviour.** It gets the same modules by name.

### Time

- **In game:** a world starts at 06:00 on day 1. One tick is 3600 in-game ms, so `os.epoch()` advances 72000 per real second. `os.setAlarm(h)` fires the next time the world's clock reaches hour `h`.
- **Real time:** `"utc"` and `"local"` are both UTC, because a computer has no time zone.
- **Timers** count whole game ticks of 0.05 s. `os.startTimer(0)` fires on the next tick.

### How keys reach a program (hardware: `[imports.console]` in `abi/child-abi.toml`)

- **On a Terminal**, a program that pulls events asks for raw keys (`key_capture_start` / `key_poll`). Presses and releases then come with their real codes, and Ctrl, Shift and the arrow keys are their own keys. Typed characters still come from stdin.
- **Without raw keys** (an SSH session, the GUI's Terminal app), keys are decoded from stdin. Arrows, Enter, Backspace, Tab, Delete, Home/End and PgUp/PgDn work. Ctrl+letter gives the letter's key.
- **While it isn't yet known which case applies**, typed bytes wait 100 ms for their raw keys.

### How the term draws

- **Drawing:** the term writes the console's escape sequences (ecm-vte).
- **Print-only programs** move only within the line and down, so they follow on from the prompt, and the console's cursor stays as it was.
- **Programs that set the cursor or clear the screen** position absolutely, and get CC's cursor: hidden unless `setCursorBlink(true)`.
- **Spawn:** the computer gives a program its cursor (`ECM_CURSOR`) along with `COLUMNS`/`LINES`.
- **On exit:** the default colours and a visible cursor come back.

### Not yet

- **Colours on screen:** the console has a fixed 16-colour palette, so CC colours show as the nearest console colour (orange and brown share one). `setPaletteColour` is remembered and read back, but doesn't change what's drawn.
- **Mouse:** `mouse_click`, `mouse_scroll` and the other mouse events are missing, because the Terminal only reports the mouse in graphics mode.
- **Terminate:** Ctrl+T stops the program at once, so a program never sees a `terminate` event from the keyboard. `os.queueEvent("terminate")` works.
- **Uptime:** `os.clock()` counts from the program's start, not the computer's.

## Later phases

1. `fs` and `io` with root-relative paths, `textutils`, `parallel`, peripheral camelCase aliases, `rs` / `redstone`.
2. Our own generic peripherals over NeoForge item, fluid and energy capabilities.
3. A floppy Disk Drive and disks of our own, plus reading CC floppies when CC is installed.
4. An optional CC peripheral bridge (`compat/computercraft`, compile-only, only loaded when CC is present).
5. A Visual Programmer peripheral catalog that learns peripheral types at runtime.

Tests: TESTING.md, "Console programs". Scenarios: `cc_lua`, `console_python` (`ecm_console`).
