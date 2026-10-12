# Visual Programmer

The Visual Programmer is the no-code path from a blank canvas to a published app ([ROADMAP P6](ROADMAP.md#p6--visual-programmer-as-an-app)). It used to be a Java screen (`VisualProgrammingScreen`) that typed `python visual_program.py` into the player's terminal. It is now `visual`, an ordinary `ecm-ui` app that saves through the file system and runs programs as normal processes.

## Overview

| Part | Where |
| --- | --- |
| The app | `rust/wasm-programs/visual` (library `ecm_visual`, binary `visual`) |
| Block definitions | `rust/wasm-programs/visual/blocks.json` (compiled in), plus peripheral blocks from the host's catalog |
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

**Program page.** A board holds:

- one panel per block, with a header in its category color and the block label;
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

Zoom levels are the `ui` font sizes 8, 12, 16 and 24 (67 %, 100 %, 133 %, 200 %), so text stays sharp. The old continuous zoom snaps to the nearest level.

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
| a peripheral block | `peripherals:<type>` |
| Set Redstone / On Redstone | `peripherals:redstone` |
| Notify | `notifications` |
| blocks named `net_*` / `http_*`, or whose code opens sockets | `network` |

There are no network blocks yet.

If the computer has `pkg` (`server-bin/pkg.wasm` or `bin/pkg.wasm`), Package runs `pkg build /visual/apps/<id>` in the run panel and says to publish with `pkg serve`. Otherwise it writes the folder and says so. `visual --package` does the same from the console, without the `pkg` step.

The schema follows [Display Pipeline §8](DISPLAY_PIPELINE.md#package-format-proposed). `icon` is an addition to the proposed format: the icon file's name.

## Toolkit additions

These live in `ecm-ui` and are also accepted in `.ui.json`.

| Kind | Props | Behaviour |
| --- | --- | --- |
| `board` | `fill`, `radius`, `border_w`, `border_color`, `capture`, `on_pointer`, `on_key` | Clips its children. Children with `x` / `y` sit at board coordinates at their own size; others fill the board. A press on the board or on a non-interactive child grabs the pointer: `on_pointer` gets Down, then Move and Up while the button is held, then Scroll, in board coordinates (`EventValue::Pointer`), with `target` set to the id of the innermost element with an id under the pointer. Interactive children keep their own presses unless `capture` is set, in which case every press inside is reported and every element gets a hit rect. Wheel steps over a scroll view inside a board scroll it. A pressed board takes focus and reports keys to `on_key` (`EventValue::Key`). |
| `panel` | `fill`, `radius`, `border_w`, `border_color`, `padding` | A hit-testable rectangle holding its children like a stack. |
| `wire` | `points` (`[[x, y], …]`, at most 256), `stroke`, `color` | One `line` node, with points relative to the parent. |

The Python module passes pointer and key values as dicts:

- Pointer: `{"kind", "x", "y", "button", "buttons", "scroll", "target"}`
- Key: `{"key", "mods"}`

`_ui.widget_state(h, win, id)` reads a widget's live state.

## Tests

| Run | Checks |
| --- | --- |
| `-Rust ecm-ui` | Board placement, line nodes, grabs with board coordinates and targets, interactive children, capture mode, keys, scroll views in boards, layout-file props (`tests/board.rs`). |
| `-Rust visual` | Codegen goldens against Java (9 graphs, byte-identical); save and reload; block registry and peripheral blocks against the Java catalog fixture; geometry. The app on a mock compositor (`tests/app.rs`): canvas tree golden; pointer placing, wiring, dragging, typing, type tags, zoom, disconnecting and deleting; Save / Open / Run through files and a runner with stdin; the old error for a graph without Start; the designer driven by pointer (drop, move, rename in the inspector, event button → entry block and synced `on_click`); the counter app's Python, layout and `app.toml` goldens; Package with and without `pkg` and inferred permissions. Plus the console CLI. |
| `-Rust python` | The designer-built counter app runs through `import ui` on a mock connection. Two clicks make the label "Count: 2". |
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
- **Redstone permission:** `peripherals:redstone` is a proposal. Redstone side I/O is not a `periph_*` call today.
