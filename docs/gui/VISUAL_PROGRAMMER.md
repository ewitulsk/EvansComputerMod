# Visual Programmer

The Visual Programmer is the no-code path from a blank canvas to a published app ([ROADMAP P6](ROADMAP.md#p6--visual-programmer-as-an-app)). It used to be a Java screen (`VisualProgrammingScreen`) that typed `python visual_program.py` into the player's terminal. It is now `visual`, an ordinary `ecm-ui` app that saves through the file system and runs programs as normal processes.

## Overview

| Part | Where |
| --- | --- |
| The app | `rust/wasm-programs/visual` (library `ecm_visual`, binary `visual`) |
| Block definitions | `rust/wasm-programs/visual/blocks.json` (compiled in: ports, code, and each block's `description` and `example` for its info popup; `collapsed` on the extra categories), plus peripheral blocks from the host's catalog |
| Toolkit additions | `ecm-ui`: `board`, `panel`, `wire`, pointer and key events (`crates/ecm-ui/src/window.rs`) |
| Python helpers | `ui.App.layout(win)` with `set` / `get` / `value` by widget id (`wasm-programs/python/src/ui.py`) |
| Runs programs | kernel command sessions, `ipc_spawn_command` (`operating-system/rust/src/sessions.rs`) |
| Peripheral catalog | `periph_methods("@catalog")` → `computer/peripheral/PeripheralCatalog.java` |
| Scenarios | `/ecm scenario spawn visual_console` and `visual_app` (`testing/scenario/VisualScenarios.java`); GameTest namespace `ecm_visual`; simulator scenario `23_visual` |

```text
visual [FILE.vpl | FILE.ui.json]   open the editor (GUI)
visual --install-samples           write sample programs into /visual
visual --python FILE.vpl           print the Python a program generates
visual --run FILE.vpl              run a program here and show its output
visual --package FILE.vpl          write a GUI app's folder under /visual/apps
```

The editor needs the GUI. In console mode `visual` prints *"the Visual Programmer is a GUI app. Start the GUI with `gui`…"*. The other forms work in the console. The kernel's old `visual` builtin is gone, so `visual` resolves to `server-bin/visual.wasm` like any program.

## Program structure

| Module | Does |
| --- | --- |
| `blocks` | Parses `blocks.json`. Builds one category per peripheral type and one block per method from the catalog, exactly as the old `VisualBlockRegistry` did: inputs `flow`, `name` and the parameters; a `result` output for non-void methods; template `(peripheral.wrap({name}) or peripheral.find("type")).method(...)`; label from the description or the capitalised name; color from Java's `String.hashCode` hue. |
| `graph` | The node graph and the `.vpl` format. Handles wiring rules, delete, bring-to-front, and port ids (`port:<block>:<o\|i>:<name>`). |
| `codegen` | Graph → Python. |
| `geometry` | Block, port and field positions and wire curves at a zoom level. |
| `designer` | The `.ui.json` document, palette, drop, move, inspector props, handler sync and the live preview. |
| `package` | App folders, `app.toml`, permissions and the icon PNG. |
| `editor` | All editor state and actions, with no UI code. It's what the tests drive. |
| `view` | Draws the editor as `ecm-ui` elements. |
| `files`, `runner` | Storage (`HostFiles` / `MemFiles` / `DirFiles`) and running programs (`SessionRunner` / `MockRunner`), behind traits. |
| `cli`, `samples` | The console commands and the sample programs, including the designer-built counter app. |

## Files

Programs live in `/visual` in the computer's storage:

- `<name>.vpl` is the graph.
- `<name>.ui.json` is its layout, for GUI apps.
- `<name>.py` is what Run executes.
- `apps/<id>/` is what Package writes.

**`.vpl` is the old editor's JSON, unchanged.** Every existing file opens:

```json
{"version":1,"name":"hello","canvas":{"offsetX":0.0,"offsetY":0.0,"zoom":1.0},
 "blocks":[{"id":1,"type":"start","x":40.0,"y":40.0,"inputs":{}}, ...],
 "connections":[{"fromBlock":1,"fromPort":"flow","toBlock":2,"toPort":"flow"}, ...]}
```

Loading works like the old screen:

- Ids are renumbered from 1 in file order.
- Unknown block types are skipped, along with their wires.
- Inputs start at their defaults, then saved values override them.
- Only values that differ from a default are saved.

Two optional keys are new, and the old editor ignored keys it didn't know:

- `"layout"`: the `.ui.json` path.
- `"app"`: `{"id", "name", "version"}`.

The old editor stored programs in the same `/visual` folder through server packets, so a world's existing programs show up in Open.

## Code generation

`codegen::generate` is a line-by-line port of `VisualCodeGenerator`:

- **Flow:** the chain from Start, plus `on_key_press` / `on_redstone` handlers, followed by indentation.
- **Branches and loops:** `if` / `elif` / `else` (a non-else false branch is wrapped in `else:`), `loop` / `for_loop` / `loop_forever`.
- **Data blocks:** emitted once, just before their first consumer.
- **Pass-through constants:** followed through to their value.
- **Quoting:** `string` inputs and `any` inputs tagged Txt are quoted. Int and Dec tags pass raw.
- **Copied quirks:** the per-branch visited sets, and `"total" = 0` for an untagged Set Variable name.

**Goldens.** Before the Java code was deleted, a one-off JUnit test (`VisualGoldenExportTest`, in this branch's history) ran the old generator over nine sample graphs in `tests/vpl/`. The samples cover every built-in block kind, renumbered ids, an unknown block, escapes, events and a test peripheral. The test committed its output as `tests/vpl/<name>.py`. `tests/golden.rs` checks that the Rust port produces **the same bytes for all nine**, and that save → reload keeps them. Line endings are pinned (`tests/.gitattributes`).

**GUI apps.** A graph that uses an App block (*When Widget*, *Set Widget Text*, *Read Widget*, *Enable Widget*, *Check Widget*, *Set Slider*, *Notify*, *Close App*) generates a `ui` program around the same chains:

```python
import ui

_app = None
_layout = None

def _on_add_click_1(e):
    global count
    _block1_value = e.value
    _block6_result = count + 1
    count = _block6_result
    _block7_result = str(count)
    _block8_result = str("Count: ") + str(_block7_result)
    _layout.set("count", text=str(_block8_result))

_app, _win = ui.load("/visual/counter.ui.json", {"add_click": _on_add_click_1}, app_id="dev.visual.counter")
_layout = _app.layout(_win)
count = 0
_app.run()
```

- Each *When Widget* block becomes a handler function. Its widget and event name the handler (`add_click`), and its `value` output is the event's value.
- Variables that Set Variable blocks assign by name are declared `global` in handlers.
- The Start chain runs after the layout is loaded, then the event loop runs.
- A packaged app loads `main.ui.json` from beside `main.py`. `python` now sets `__file__`, and `ui.load` falls back to the computer's storage when `open()` can't read a path.

## The editor

**Program page.** On the left, the **block palette** (170 lu wide, a scroll view):

- One section per category under a header button: `- Control` when open, `+ Redstone (1)` when
  folded (with its block count). Clicking a header opens or folds it, and the choice lasts while
  the editor runs (a New program keeps it).
- **Open at start:** the core categories, Control, Output, Timing, Math, Compare and Constants.
  **Folded at start:** the extra ones, Redstone, Events, Convert and App (`"collapsed": true` in
  `blocks.json`) and every peripheral category from the catalog (there can be many, with long
  method lists). Core = what a first program needs; the rest is one click away.
- Each block's button arms it for placing (its label is cut with `...` if the palette is too
  narrow), and the `?` beside it opens its **info popup**.
- The scrollbar can be dragged and its track clicked (a page at a time), as well as the wheel.

The board holds:

- one panel per block, with a header in its category color, the block label and a round `?`
  **info button** in the header's right corner;
- port dots colored by type (flow white, string teal, number blue, any yellow);
- inline text fields for unconnected inputs;
- option fields (click to cycle) and `[Txt]` / `[Int]` / `[Dec]` tags on `any` inputs;
- wires as `line` nodes (cubic curves, 20 segments).

Controls:

- **Place:** pick a block in the palette, then click the canvas.
- **Wire:** drag from a port to a port. The old rules apply: output to input only, types must match (`any` matches all), an input takes one wire, and a flow output starts one wire.
- **Move:** drag a block. It comes to the front.
- **Remove:** right-click a port to cut its wires, or right-click a block (or press Delete with it selected) to delete it.
- **View:** drag empty canvas or middle-drag to pan. Use the wheel or the − / + buttons to zoom.
- **Cancel:** Esc cancels placing or a drag.
- **Info:** the `?` on a block (or beside it in the palette) opens a popup with the block's full
  name, what it does, one line per input (type, default, choices) and output (what a flow output
  runs), an example, and the Python it stands for. Peripheral blocks get theirs from the catalog
  (method and type descriptions, how `name` finds the peripheral). Close shuts it.

**Readable blocks.** Every text on a block is measured with the ECM font metrics
(`geometry::block_geom`), at every zoom:

- A title wider than 200 lu (at 100 %) wraps onto a second line and the header grows; a longer
  one is cut with `...` on that line. The info button shows all of it.
- The block is at least as wide as its title plus the info button, its inputs (name, type tag,
  field) side by side with its outputs, and, with several flow outputs, a slot per label.
- Value fields grow with their text up to 160 lu; a text field scrolls beyond that, an option
  field cuts its label with `...`.
- `tests/readability.rs` draws every block (blocks.json, the catalog fixture and a peripheral
  with very long names) at 67, 100, 133 and 200 %, with default and with long values, through
  the real view, and checks that no text leaves its block (titles their header), no two texts
  or fields overlap, and text on a panel (type tags, option fields, the `?`) stays on it.

Zoom levels are the `ui` font sizes 8, 12, 16 and 24 (67 %, 100 %, 133 %, 200 %), so text stays sharp. The old continuous zoom snaps to the nearest level.

The inspector on the Design page is a scroll view in a board too; its scrollbar drags the same way.

**Toolbar:**

- Program / Design
- the name field
- New, Open (lists `/visual/*.vpl` and `*.ui.json`), Save (needs a name, like the old editor)
- Run / Stop
- Package
- zoom

**Run panel.** Run opens a panel at the bottom with the program's last lines of output (control sequences stripped). It has a field that sends a line to the program's input, so the Input block works, and Stop / Close buttons.

## Designer

**Design page.** A capturing board holds the widget palette and a live preview of the layout, drawn by the real toolkit. Every press inside reports which widget it hit, by path, so widgets in the preview are selected instead of used.

- **Add:** drag a widget from the palette. Dropped on a container (row, column, stack, grid, panel, scroll) it goes inside; dropped on anything else it goes after that widget.
- **Move:** drag a widget in the preview. A widget can't go into itself.
- **Select:** click a widget. It gets an outline that lays out like the widget.
- **Delete:** the Delete key, or the Delete button.

**Inspector.** It shows the props for the selected widget's kind:

- id, text, placeholder, style, size, color, gap, padding, align, columns, min / max / value / step, checked, enabled, wrap, width, height, grow.

Invalid values leave the widget unchanged. Renaming an id renames it in the graph's widget blocks too. With nothing selected, the inspector edits the window title and the app's id, name and version.

**Events become entry points.** For each event the widget sends, the inspector has an *Add blocks / Edit blocks when clicked / changed / submitted / selected* button. It finds or creates the *When Widget* block for that widget and event, then switches to the program with that block centred. The designer never asks for handler names: on save, run and package, `sync_handlers` sets each control's `on_<event>` to `<id>_<event>` for the events a block listens to, and removes the rest.

## Run

Run writes `/visual/<name>.py` (and the layout, for GUI apps), then starts `python /visual/<name>.py` with `ipc_spawn_command`. That is a **command session**: the kernel shell runs one command line with its own terminal buffer, no banner or prompt, and marks the session exited when the command ends. Programs started from it are ordinary jobs, children of the session. Nothing is typed into the player's terminal.

The same mechanism serves `visual --run` in the console and Package's `pkg build`. It is available to any program through `ecm_host_abi::ipc::spawn_command` (Java `WasiFunctions`, the simulator and the kernel's `SESSION_SPAWN` with an optional command blob).

## Package and publish

Package saves, then writes `/visual/apps/<id>/`:

- `app.toml`
- `main.py` (generated, layout next to it)
- `main.ui.json` (handlers synced)
- `icon.png` (32×32: the name's initial on a rounded square)

```toml
# Built by the Visual Programmer from visual/counter.vpl
id          = "dev.visual.counter"
name        = "Counter"
version     = "1.0.0"
entry       = "main.py"
icon        = "icon.png"
sizes       = ["compact", "medium", "wide"]
permissions = []
```

**Permissions are inferred from the blocks:**

| Blocks used | Permission |
| --- | --- |
| a peripheral block | `peripherals:<type>`; radio hardware (`sdr`, `amplifier`, `antenna`, `dish`, `microwave_radio`, `wifi`) → `radio`, modems → `cellular`, controllers → `controller`, as the app platform checks them ([APPS.md](APPS.md) on the P4 branch) |
| Set Redstone / On Redstone | `peripherals:redstone` |
| Notify | `notifications` |
| blocks named `net_*` / `http_*`, or whose code opens sockets | `network` |

There are no network blocks yet.

If the computer has `pkg` (`server-bin/pkg.wasm` or `bin/pkg.wasm`), Package runs `pkg build /visual/apps/<id>` in the run panel and says to publish with `pkg serve`. Otherwise it writes the folder and says so. `visual --package` does the same from the console, without the `pkg` step.

The schema follows [Display Pipeline §8](DISPLAY_PIPELINE.md#package-format-proposed) and the P4 lane's `docs/gui/APPS.md`, which also has `icon` and the strict key set this uses. `pkg build <dir>` and `pkg serve <dir> [port]` are that lane's commands. Installed apps run as `python /apps/<id>/main.py`, so `_here` finds the layout.

## Toolkit additions

These live in `ecm-ui` and are also accepted in `.ui.json`.

| Kind | Props | Behaviour |
| --- | --- | --- |
| `board` | `fill`, `radius`, `border_w`, `border_color`, `capture`, `on_pointer`, `on_key` | Clips its children. Children with `x` / `y` sit at board coordinates at their own size; others fill the board. A press on the board or on a non-interactive child grabs the pointer: `on_pointer` gets Down, then Move and Up while the button is held, then Scroll, in board coordinates (`EventValue::Pointer`), with `target` set to the id of the innermost element with an id under the pointer. Interactive children keep their own presses unless `capture` is set, in which case every press inside is reported and every element gets a hit rect. A scroll view's scrollbar (thumb and track) counts as interactive, so it is dragged and clicked inside a plain board too. Wheel steps over a scroll view inside a board scroll it. A pressed board takes focus and reports keys to `on_key` (`EventValue::Key`). |
| `panel` | `fill`, `radius`, `border_w`, `border_color`, `padding` | A hit-testable rectangle holding its children like a stack. |
| `wire` | `points` (`[[x, y], …]`, at most 256), `stroke`, `color` | One `line` node, with points relative to the parent. |

The Python module passes pointer and key values as dicts:

- Pointer: `{"kind", "x", "y", "button", "buttons", "scroll", "target"}`
- Key: `{"key", "mods", "down", "repeat"}` (boards report presses and repeats; custom areas releases too)

`_ui.widget_state(h, win, id)` reads a widget's live state.

## Tests

| Run | Checks |
| --- | --- |
| `-Rust ecm-ui` | Board placement, line nodes, grabs with board coordinates and targets, interactive children, capture mode, keys, scroll views in boards, layout-file props (`tests/board.rs`). |
| `-Rust visual` | Codegen goldens against Java (9 graphs, byte-identical); save and reload; block registry and peripheral blocks against the Java catalog fixture; every block has a description and an example; which categories start folded; geometry; block text within its block at every zoom (`tests/readability.rs`); palette sections folding (remembered across New), the info popup from the palette and from a placed block, the palette's thumb drag and track click (`tests/app.rs`). The app on a mock compositor (`tests/app.rs`): canvas tree golden; pointer placing, wiring, dragging, typing, type tags, zoom, disconnecting and deleting; Save / Open / Run through files and a runner with stdin; the old error for a graph without Start; the designer driven by pointer (drop, move, rename in the inspector, event button → entry block and synced `on_click`); the counter app's Python, layout and `app.toml` goldens; Package with and without `pkg` and inferred permissions. Plus the console CLI. |
| `-Rust python` | The designer-built counter app runs through `import ui` on a mock connection. Two clicks make the label "Count: 2". |
| `-JUnit UiGoldenTest -McVersion 26.1` | Golden images `visual_program` (palette with folded sections, a two-line peripheral title, the info buttons) and `visual_info` (Set Redstone's popup), recorded with `cd rust; cargo run -p visual --example record_visual_sessions -- ../src/test/resources/ui-sessions`. |
| `-Scenarios visual` | Simulator: install samples, `--python`, `--run` of hello and counter_loop (real `python` processes in command sessions), the GUI-app refusal, `--package`, and the packaged `main.py` reaching its console-mode error after loading its layout. |
| `-GameTests ecm_visual` | `visual_console` on a player-built Terminal (24 s). |
| `-Rust terminal-os` | Command sessions end with their command, with no prompt. |
| `-JUnit KernelHostIntegrationTest -McVersion 26.1`, `scripts/check-abi.py` | The kernel without `open_visual_editor`. |

## Status and gaps

- **Needs the compositor (P2) to verify in-game.**
  - The editor window itself.
  - `visual_app`: `gui`, `gui launch visual …`, the Counter window from the packaged app, and clicking it.
  - In this base, child programs get zero-returning stubs for unknown imports. So `visual` without arguments reports a UI protocol error instead of the console message until `ui_open` exists (it returns -4 in console mode then).
  - The scene was checked with a scratch rasterizer from the TreeOps the mock received, not the Java one.
- **Needs `pkg` (P4).**
  - `pkg build` / `pkg serve` and the App Store install on a second computer: the second "Done when" box.
  - Package detects `pkg` by its file in `server-bin/` or `bin/` through the storage host calls. Whether `server-bin` is visible there needs checking once `pkg` exists.
- **The file picker and "open with"** (UI_PROTOCOL §12) aren't used yet. Open lists `/visual`. `OpenFile` messages are handled, so "open with" works once Files sends them.
- **Not ported:** the ghost block that followed the mouse before placing (placing is click-to-arm, then click), double-click to edit (fields are always inline), and continuous zoom.
- **Redstone permission:** `peripherals:redstone` matches the P4 lane's check on `redstone_*` host calls.
