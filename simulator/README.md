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
Files are stored in the `--storage` directory (default `./simulator-data/`). All file operations work identically to the Minecraft mod: `write_file`, `read_file`, `file_exists`, `file_size`, `delete_file`, `list_files`.

### Python REPL
The embedded RustPython interpreter works fully. Start it with the `python` command, or run scripts with `python <filename>`.

```
> python
Python 3.11 (RustPython)
>>> import terminal
>>> terminal.println("Hello from Python!")
Hello from Python!
>>> terminal.write_file("test.py", "import terminal\nterminal.println('It works!')\n")
>>> exit()
> python test.py
Running: test.py

It works!
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
    main.rs                 -- CLI args, threading, raw terminal input
    wasm_host.rs            -- Wasmtime engine, Linker, module loading, worker loop
    host_functions.rs       -- All 23 host function implementations
    terminal_io.rs          -- 80x24 screen buffer, crossterm rendering
    filesystem.rs           -- File I/O with path sanitization
    redstone.rs             -- Simulated redstone state (6 sides)
    interrupts.rs           -- Thread-safe interrupt queue
    wasm_bindgen_stubs.rs   -- Dynamic prefix-based stub registration (~50 imports)
```

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

- **`print()` in Python**: The built-in `print()` function shows "RuntimeError: lost sys.stdout" because RustPython in WASM doesn't have real file descriptors. Use `terminal.println()` instead. This is the same behavior as in the Minecraft mod.
- **Visual editor**: The `visual` command prints a stub message since the visual programming UI is a Minecraft client-side screen.
- **Arrow keys**: Not forwarded to the WASM OS (same as the Minecraft mod's terminal).

## Dependencies

- **wasmtime 29** — WASM runtime (Rust crate)
- **crossterm 0.28** — Cross-platform raw terminal I/O
- **clap 4** — CLI argument parsing
- **rand 0.8** — Random number generation for `__getrandom_v03_custom`
- **chrono 0.4** — Timezone offset for wasm-bindgen date stubs
