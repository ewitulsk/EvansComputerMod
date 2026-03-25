# Terminal OS Simulator

A standalone CLI simulator that runs the same `terminal_os.wasm` binary used by the Minecraft mod, without needing Minecraft. The existing Rust OS drops in and works without any modifications.

## Quick Start

```bash
cd simulator

# Build
cargo build --release

# Run (interactive mode)
cargo run --release -- --wasm ../wasm-bin/terminal_os.wasm

# Run (headless mode, for testing/piping)
echo "help" | cargo run --release -- --wasm ../wasm-bin/terminal_os.wasm --headless
```

## How It Works

The simulator replaces the Java/Minecraft host with a native Rust CLI program. It loads the same `terminal_os.wasm` binary and provides all the host functions the WASM module expects:

```
Python Script
    |
RustPython Interpreter (embedded in Rust OS)
    |
Rust Operating System (terminal_os.wasm — unchanged)
    |
Wasmtime (Rust crate)          <-- simulator replaces wasmtime-java
    |
CLI Terminal (crossterm)        <-- simulator replaces Minecraft renderer
```

The simulator implements every host function that the Java `TerminalWasmHost` provides, including ~50 wasm-bindgen stubs required by RustPython's dependencies. The stubs are matched dynamically by prefix pattern, so they continue to work even if the WASM binary is recompiled with different wasm-bindgen hashes.

## CLI Options

```
terminal-simulator [OPTIONS]

Options:
      --wasm <PATH>      Path to terminal_os.wasm [default: ../wasm-bin/terminal_os.wasm]
      --storage <DIR>    File storage directory [default: ./simulator-data]
      --width <N>        Terminal width [default: 80]
      --height <N>       Terminal height [default: 24]
      --display <WxH>    Enable pixel display (e.g., --display 64x64)
      --headless         Run without raw terminal (reads lines from stdin)
  -h, --help             Print help
```

## Keyboard Shortcuts (Interactive Mode)

| Key | Action |
|-----|--------|
| Ctrl+Q | Quit the simulator |
| Ctrl+T | Terminate current program (IRQ 15) |
| Ctrl+C | Terminate current program (same as Ctrl+T) |
| Ctrl+R | Enter redstone input mode |
| Ctrl+D | Send EOF (exits Python REPL) |

All other keys are forwarded to the WASM OS as-is.

## Features

### Terminal I/O
Full 80x24 terminal emulation with cursor positioning, screen clearing, and text rendering via crossterm. The WASM OS handles its own line editing, command parsing, and text editor.

### File System
Supports nested directories. Files are stored in the `--storage` directory (default `./simulator-data/`).

Available commands: `ls`, `cd`, `pwd`, `mkdir`, `touch`, `cat`, `cp`, `mv`, `rm`, `edit`.

```
/ > mkdir src
/ > touch src/main.py
/ > ls
  src/
  readme.txt
/ > ls -l
  d  ---     src/
  f       0B readme.txt
/ > cd src
/src > pwd
/src
/ > rm -r src
```

### Python REPL
The embedded RustPython interpreter works fully. Python's built-in `print()` works, and expression results display in the REPL.

```
/ > python
Python 3.11 (RustPython)
>>> print("Hello from Python!")
Hello from Python!
>>> 2 + 2
4
>>> exit()
/ > python test.py
Running: test.py
```

### Redstone Simulation
Redstone output calls are logged to stderr. To simulate redstone input, press Ctrl+R in interactive mode and enter `<side> <power>`:

```
[Redstone] Enter 'side power' (e.g., '3 15' for BACK=15), or 'q' to cancel:
3 15
[Redstone] Set side 3 = 15
```

Side constants: 0=DOWN, 1=UP, 2=FRONT, 3=BACK, 4=LEFT, 5=RIGHT

Setting redstone input queues an IRQ_REDSTONE (2) interrupt, which Python scripts can handle:

```python
import terminal

def on_redstone(data):
    terminal.println("Redstone changed: " + str(data))

terminal.on_interrupt(terminal.IRQ_REDSTONE, on_redstone)

while True:
    terminal.check_interrupts()
    terminal.sleep(0.1)
```

### Interrupt System
The simulator implements the full interrupt system:
- **IRQ 1 (Keyboard)**: Keyboard input events during execution
- **IRQ 2 (Redstone)**: Redstone input changes (via Ctrl+R)
- **IRQ 15 (Terminate)**: Ctrl+T / Ctrl+C, non-maskable, resets to shell

### Pixel Display Simulation

The simulator can emulate a pixel display for testing the `display` Python module without Minecraft. Enable it with `--display WxH`:

```bash
# 64x64 pixel display
cargo run --release -- --wasm ../wasm-bin/terminal_os.wasm --display 64x64

# Headless testing
echo "python test_framebuffer.py" | cargo run --release -- --headless --display 64x64 2>/dev/null
```

When enabled, the simulator provides a `SimFramebuffer` that implements all 8 `fb_*` host functions:

| Host Function | Simulator Behavior |
|---------------|-------------------|
| `fb_get_width` / `fb_get_height` | Returns configured dimensions |
| `fb_set_pixel` | Sets pixel in memory buffer with bounds checking |
| `fb_fill_rect` | Fills rectangle in memory buffer |
| `fb_write_region` | Bulk writes RGBA data from WASM memory |
| `fb_clear` | Fills entire buffer with color |
| `fb_flush` | Computes dirty 16x16 tiles, returns count |
| `fb_blit_text` | Renders text using 8x16 character cells (simple rectangles) |

#### Display Rendering

In interactive mode, the display is rendered to the terminal using ANSI half-block characters (`▀`). Each terminal cell represents 2 vertical pixels using 24-bit foreground/background colors. A 64x64 display uses approximately 32 terminal rows.

In headless mode, display output is suppressed (but flush counts are still returned to scripts for testing).

#### Dirty-Tile Tracking

The simulator implements the same dirty-tile diffing as the Java server:

1. Drawing operations modify the `pixels` buffer
2. `fb_flush()` compares `pixels` against `prev_pixels` in 16x16 tile blocks
3. Returns the count of actually-changed tiles
4. Updates `prev_pixels` snapshot for the next flush

This means writing the same color twice produces 0 dirty tiles on the second flush — identical behavior to the Minecraft mod.

#### Test Script

A comprehensive test suite is included at `test-data/test_framebuffer.py` (226 lines, 29 tests). To run it:

```bash
# Copy test to simulator storage, then run
cp test-data/test_framebuffer.py simulator-data/test_framebuffer.py
echo "python test_framebuffer.py" | cargo run --release -- --headless --display 64x64 2>/dev/null
```

Tests cover: display attachment, clear, set_pixel, fill_rect, rect outline, lines, text rendering, idempotent operations, boundary conditions, batched operations, and overwrite detection.

### Peripherals
Peripheral host functions are stubbed to return empty results (no Minecraft peripherals available in the simulator). The `peripherals` command and `peripheral.list()` in Python will show no connected peripherals.

## Headless Mode

Headless mode (`--headless`) reads lines from stdin instead of using raw terminal input. This is useful for automated testing and CI:

```bash
# Run a sequence of commands
printf 'help\nls\npython\nimport terminal\nterminal.println("test")\nexit()\n' \
  | cargo run --release -- --headless 2>/dev/null

# Pipe a script
echo 'python test.py' | cargo run --release -- --headless 2>/dev/null
```

## Project Structure

```
simulator/
  Cargo.toml
  src/
    main.rs                 -- CLI args, threading, raw terminal input, --display flag
    wasm_host.rs            -- Wasmtime engine, Linker, HostState (with custom state)
    host/                   -- Host function modules (one file per group)
      mod.rs                -- Registration hub: register_all() + known_names()
      memory.rs             -- Public WASM memory helpers (read_string, read_bytes, write_bytes)
      terminal.rs           -- terminal_write, terminal_clear, etc.
      filesystem.rs         -- file_write, file_read, etc.
      redstone.rs           -- redstone_set_output, redstone_get_input, etc.
      sleep.rs              -- sleep_ms
      interrupts.rs         -- interrupt_poll, interrupt_poll_len
      peripherals.rs        -- peripheral_list, peripheral_get_methods, peripheral_call
      framebuffer.rs        -- fb_* host functions + SimFramebuffer state
      getrandom.rs          -- __getrandom_v03_custom
    terminal_io.rs          -- 80x24 screen buffer, crossterm rendering
    display_renderer.rs     -- ANSI half-block framebuffer rendering + PPM export
    filesystem.rs           -- File I/O with path sanitization
    redstone.rs             -- Simulated redstone state (6 sides)
    interrupts.rs           -- Thread-safe interrupt queue
    wasm_bindgen_stubs.rs   -- Dynamic prefix-based stub registration (~50 imports)
  test-data/
    test_framebuffer.py     -- Comprehensive display test suite (29 tests)
```

## Adding Host Functions

Every host function module follows the same pattern. To add a new one:

### Step 1: Create your module file

Create a new file in `src/host/`, for example `src/host/my_sensor.rs`:

```rust
//! Host functions for my custom sensor peripheral.

use wasmtime::*;
use crate::wasm_host::HostState;
use super::memory;

/// Host function names registered by this module.
pub const FUNCTIONS: &[&str] = &[
    "sensor_get_value",
    "sensor_get_name",
];

pub fn register(linker: &mut Linker<HostState>) -> Result<()> {
    // A simple function that returns a value from custom state
    linker.func_wrap("env", "sensor_get_value", |caller: Caller<'_, HostState>| -> i32 {
        // Access custom state (see step 3)
        caller.data()
            .get_custom::<SensorState>()
            .map(|s| s.value)
            .unwrap_or(0)
    })?;

    // A function that writes a string back to WASM memory
    linker.func_wrap("env", "sensor_get_name",
        |mut caller: Caller<'_, HostState>, buf_ptr: i32, buf_len: i32| -> i32 {
            let name = b"simulated_sensor";
            let write_len = name.len().min(buf_len as usize);
            memory::write_bytes(&mut caller, buf_ptr, &name[..write_len]);
            write_len as i32
        },
    )?;

    Ok(())
}

/// Custom state for this module.
pub struct SensorState {
    pub value: i32,
}
```

### Step 2: Register it in `src/host/mod.rs`

Add three lines:

```rust
mod my_sensor;  // 1. declare the module
// ...
pub fn known_names() -> Vec<&'static str> {
    // ...
    names.extend_from_slice(my_sensor::FUNCTIONS);  // 2. add names
    names
}

pub fn register_all(linker: &mut Linker<HostState>) -> Result<()> {
    // ...
    my_sensor::register(linker)?;  // 3. register functions
    Ok(())
}
```

### Step 3: Add custom state (optional)

If your host functions need persistent state, use the `custom` storage on `HostState`. In `src/wasm_host.rs`, after `HostState` is created:

```rust
// In WasmHost::new(), after creating the HostState:
state.insert_custom(my_sensor::SensorState { value: 42 });
```

Then access it from any host function via `caller.data().get_custom::<SensorState>()` or `caller.data_mut().get_custom_mut::<SensorState>()`.

### Step 4: Add the matching Rust OS side

On the WASM OS side (`operating-system/rust/`), declare the corresponding `extern "C"` functions:

```rust
extern "C" {
    fn sensor_get_value() -> i32;
    fn sensor_get_name(buf_ptr: *mut u8, buf_len: i32) -> i32;
}
```

### WASM memory helpers

The `host::memory` module provides three functions for exchanging data with WASM:

| Function | Use for |
|----------|---------|
| `memory::read_string(&mut caller, ptr, len)` | Reading a string argument from WASM (e.g., a filename) |
| `memory::read_bytes(&mut caller, ptr, len)` | Reading raw bytes from WASM (e.g., file content) |
| `memory::write_bytes(&mut caller, ptr, bytes)` | Writing data back to a WASM buffer |

### Reference examples

| Complexity | File | What it shows |
|-----------|------|---------------|
| Minimal | `host/sleep.rs` | Simplest possible host function (no memory access) |
| Medium | `host/redstone.rs` | Reading shared state + writing bytes to WASM memory |
| Full | `host/filesystem.rs` | Reading strings and bytes from WASM, writing back results |

## Threading Model

The simulator mirrors the Java `TerminalWasmHost` worker thread architecture:

```
Main Thread                          WASM Worker Thread
-----------                          ------------------
raw terminal mode (crossterm)        load terminal_os.wasm
read keystrokes                      call main() -> boot banner
  |                                  loop {
  +--> input channel --------+         drain interrupt queue
  |                          +-------> poll input (100ms timeout)
  +--> interrupt queue ---+            call on_input(ptr, len)
       (Ctrl+T -> IRQ 15) +---------> drain interrupts
       (Ctrl+R -> IRQ 2)            }
```

## Known Limitations

- **Visual editor**: The `visual` command prints a stub message since the visual programming UI is a Minecraft client-side screen.
- **Arrow keys**: Not forwarded to the WASM OS (same as the Minecraft mod's terminal).
- **64KB file size limit**: Files larger than 64KB cannot be read (host buffer limitation).
- **No pipes or redirection**: Shell does not support `|`, `>`, `<` operators.
- **Display rendering**: The half-block ANSI display rendering requires a terminal that supports 24-bit color (most modern terminals do). In headless mode, display output is not rendered but `flush()` still returns correct tile counts for testing.
- **Display text**: `fb_blit_text` in the simulator uses simple rectangle fills for character cells rather than actual glyph rendering. Characters appear as solid 8x16 blocks.

## Dependencies

- **wasmtime 29** — WASM runtime (Rust crate)
- **crossterm 0.28** — Cross-platform raw terminal I/O
- **clap 4** — CLI argument parsing
- **rand 0.8** — Random number generation for `__getrandom_v03_custom`
- **chrono 0.4** — Timezone offset for wasm-bindgen date stubs
