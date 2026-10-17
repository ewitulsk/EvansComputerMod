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

## Phase 2: files, text, redstone, parallel (done)

| CC | Built on (Lua and Python) | Notes |
|---|---|---|
| `fs.*` (`list`, `exists`, `isDir`, `isReadOnly`, `getSize`, `getFreeSpace`, `getDrive`, `getCapacity`, `isDriveRoot`, `makeDir`, `move`, `copy`, `delete`, `combine`, `getName`, `getDir`, `find`, `attributes`, `open`) | `fs` (snake_case in both) | Paths are from the computer's root, with or without a leading `/`. Errors use CC's wording (`/x: No such file`, `/x: File exists`, `/..: Invalid Path`). `fs.open` returns CC's handles (`readLine`, `readAll`, `read`, `write`, `writeLine`, `flush`, `seek`, `close`), or `nil` plus the reason. `getFreeSpace` is `"unlimited"`. |
| `io.open`, `io.lines` | Lua's `io` | Already root-relative: programs start with the computer's root as their working directory. |
| `textutils.serialize` / `serialise`, `unserialize`, `serializeJSON`, `unserializeJSON`, `formatTime`, `urlEncode`, `tabulate`, `pagedTabulate`, `pagedPrint`, `slowWrite`, `slowPrint`, `empty_json_array` | `textutils` | The layout is CC's: two-space indent and a comma after every entry. `{compact = true}` gives one line. Map keys are sorted, so the output is the same every run. A table inside itself, or a function, is an error, as in CC. `unserialize` reads literals only. JSON writes 1..n tables as arrays and other tables as objects. |
| `rs` / `redstone` (`getSides`, `getInput`, `setOutput`, `getOutput`, `getAnalogInput` / `Analogue`, `setAnalogOutput`, `getAnalogOutput`, the bundled functions) | `redstone` | Sides are named `top`, `bottom`, `front`, `back`, `left` and `right`. There are no bundled cables: bundled input reads 0 and bundled output is ignored. `getOutput` reads the computer's own output (new host function `redstone_get_output`). |
| the `redstone` event | the event queue | Fires when any input changes, while the program is pulling events. |
| `parallel.waitForAny`, `waitForAll` | Lua coroutines (Lua only) | Inside these functions, `os.pullEvent`, `sleep` and `read` yield, so the other functions keep running. Python has no coroutines that could run functions this way. |
| `peripheral.getNames`, `getType`, `isPresent`, `getMethods`, `getName`, `find(type, filter)` | `peripheral` | `find` returns every match: the first as before, the rest as extra results. Python's `find` takes the filter too. |

Errors from these functions are plain strings, as CC's are, so programs can match and print them.

## Phase 3: any inventory, tank or battery is a peripheral (done)

A block next to a computer that has no peripheral of its own, but has NeoForge's item, fluid or energy capability, now attaches as a generic peripheral. That covers a vanilla chest, a hopper or a cauldron, and any mod's machine. The block's mod needs no code. This is `computer/peripheral/generic/`, 1.21.1 only: 26.1's NeoForge has a different transfer API.

| Type (the first the block has) | Methods (snake_case; Lua also takes CC's camelCase) |
|---|---|
| `inventory` | `size()`, `list()` (`{slot = {name, count}}`), `get_item_detail(slot)` (`name`, `count`, `displayName`, `maxCount`, `damage`/`maxDamage`, `tags`), `get_item_limit(slot)`, `push_items(to, from_slot, limit?, to_slot?)`, `pull_items(from, from_slot, limit?, to_slot?)` |
| `fluid_storage` | `tanks()` (`{ {name, amount}, ... }`, mB), `push_fluid(to, limit?, fluid?)`, `pull_fluid(from, limit?, fluid?)` |
| `energy_storage` | `get_energy()`, `get_energy_capacity()` (FE) |

- **Mixed blocks:** a block with several capabilities gets the methods of each, under the first type it has, in the order above.
- **Slots** are numbered from 1 in both languages, as in CC.
- **Moving things:** `to` and `from` name another attached peripheral. Errors use CC's wording: `Target 'x' does not exist`, `Target 'x' is not an inventory`, `Slot out of range (1-27)`.
- **CC method names in Lua:** a wrapped peripheral also has camelCase aliases, so `chest.getItemDetail` calls `get_item_detail`. `peripheral.call(name, "pushItems", ...)` maps the name the same way.
- **Visual Programmer:** all three types are in its catalog.
- **Not covered:** CC's other generic sources (`getItemDetail`'s NBT hash, custom generic methods from other mods). Those arrive with the CC bridge.

## Phase 4: floppy disks and the Disk Drive (done)

- **Blocks and items:** a **Disk Drive** block and a **Floppy Disk** item, both 1.21.1 only.
  - Floppy recipe: paper plus redstone, shapeless.
  - Disk Drive recipe: seven stone and two redstone.
  - Models are made in Blockbench: `models/disk_drive.bbmodel` and `floppy_disk.bbmodel`.
- **Inserting and removing:** use a floppy on a drive to put it in, and use the drive with an empty hand to take it out. Hoppers and pipes insert and extract through the item capability. A comparator reads 15 while a floppy is in.
- **Files:**
  - While a floppy is in a drive next to a computer, its files are the folder `disk/`. A second drive gives `disk2/`, and so on.
  - Every program sees the folder: the shell (`ls`, `cd`), Lua's and Python's files (`io.open`, `fs`, `open`), and `cat`/`cp`.
  - The files live in `<world>/computer-data/disks/<n>/`.
  - A floppy gets its number the first time it goes into a drive, so copies of the item are the same disk.
  - The label is the item's name. Rename it on an anvil, or with `disk.set_label`.
- **Peripheral:** the drive is a peripheral of type `drive` with `is_disk_present`, `has_data`, `get_mount_path`, `get_disk_label`, `set_disk_label`, `get_disk_id` and `eject_disk`. Events: `disk` when a floppy goes in, `disk_eject` when it comes out.
- **`disk` module** (Lua and Python): `is_present`, `has_data`, `get_mount_path`, `get_label`, `set_label`, `get_id` and `eject`, plus no-op audio functions. Lua also has CC's names (`disk.isPresent`, `disk.getID`, ...).
- **New primitive, `StorageMounts`:** folders mounted into a computer through `IComputerAccess.mount`/`unmount`.
  - It is kept by the computer's `ComputerHost`, so mounts outlive a reboot.
  - The kernel and WASI both resolve paths through it, and read-only mounts refuse writes.
  - The root listing shows the mount points.
- **Not yet:**
  - **CC floppies:** reading CC: Tweaked's own floppies (`<world>/computercraft/disk/<id>`) when CC is installed. That needs CC in the test run, so it comes with the bridge.
  - **Size limit:** there is no per-disk size limit yet; computers have no storage quota at all.

## Later phases

1. An optional CC peripheral bridge (`compat/computercraft`, compile-only, only loaded when CC is present).
2. A Visual Programmer peripheral catalog that learns peripheral types at runtime.

Tests: TESTING.md, "Console programs". Scenarios (all in `ecm_console`): `cc_lua` and `console_python` (phase 1), `cc_world` and `console_world` (phase 2), `generic_chests` and `generic_tanks` (phase 3), `floppy_disks` (phase 4). Render check: `-ClientChecks -ClientSuite disks`.
