# EvansComputerMod

A Minecraft mod that adds in-world computer terminals powered by WebAssembly. The terminals run a Rust-based operating system with an embedded Python interpreter (RustPython), giving you a fully programmable computer inside Minecraft.

**Minecraft:** 1.21.1 | **Mod Loader:** NeoForge 21.1.77+

## Architecture

```
Python Script
    |
RustPython Interpreter (embedded in Rust OS)
    |
Rust Operating System (compiled to WASM)
    |
Wasmtime-Java (WASM runtime in the mod)
    |
Minecraft / NeoForge
```

## Getting Started

1. Place a **Terminal** block (found in the Redstone and Functional Blocks creative tabs)
2. Right-click to open the terminal
3. Type `help` to see available commands

## Shell Commands

| Command | Usage | Description |
|---------|-------|-------------|
| `help` | `help` | List all commands |
| `clear` | `clear` | Clear the screen |
| `ls` | `ls` | List saved files |
| `cat` | `cat <file>` | Print file contents |
| `edit` | `edit <file>` | Open the text editor |
| `rm` | `rm <file>` | Delete a file |
| `echo` | `echo <text>` | Print text |
| `python` | `python` | Start the Python REPL |
| `python` | `python <file>` | Run a Python script |
| `peripherals` | `peripherals` | List connected peripherals |
| `peripherals` | `peripherals <name>` | Show methods on a peripheral |

**Ctrl+T** will terminate any running program and return to the shell.

## Python

The terminal embeds a full Python interpreter. Start it with `python` for an interactive REPL, or `python script.py` to run a file.

```
> python
Python REPL (RustPython)
Type 'exit()' or 'quit()' to return to shell
>>> print("Hello, Minecraft!")
Hello, Minecraft!
>>> exit()
```

Two built-in modules are available: `terminal` and `peripheral`.

---

### `terminal` Module

#### Text Output

```python
import terminal

terminal.write("no newline")
terminal.println("with newline")
terminal.clear()
```

#### Cursor and Screen

```python
terminal.set_cursor(0, 0)        # Move cursor (x, y), 0-based
width = terminal.get_width()     # 80
height = terminal.get_height()   # 24
```

#### Sleep

```python
terminal.sleep(1.0)    # seconds, supports fractional values
terminal.sleep(0.25)
```

#### File System

Each terminal has its own persistent storage. Files survive across sessions.

```python
# Write and read
terminal.write_file("greeting.txt", "Hello!")
content = terminal.read_file("greeting.txt")   # "Hello!" or None if missing

# Check and list
terminal.file_exists("greeting.txt")            # True
terminal.file_size("greeting.txt")              # 6 (bytes) or None
files = terminal.list_files()                   # newline-separated string

# Delete
terminal.delete_file("greeting.txt")            # True/False
```

#### Redstone

Terminals can output redstone signals (0-15) on all six sides. Sides are relative to the terminal's facing direction.

```python
import terminal

# Side constants
terminal.DOWN    # 0
terminal.UP      # 1
terminal.FRONT   # 2 (screen side)
terminal.BACK    # 3
terminal.LEFT    # 4
terminal.RIGHT   # 5

# Set power level (0-15)
terminal.set_redstone(terminal.BACK, 15)   # Full power behind terminal
terminal.set_redstone(terminal.BACK, 0)    # Off
```

---

### `peripheral` Module

Terminals can interact with adjacent CC:Tweaked peripherals (requires [CC: Tweaked](https://modrinth.com/mod/cc-tweaked) to be installed).

#### Discovering Peripherals

```python
import peripheral

# List all connected peripherals
peripherals = peripheral.list()
for p in peripherals:
    print(f"{p['name']} ({p['type']}) on {p['side']}")

# Get just the names
names = peripheral.get_names()   # ["chat_box_0", "player_detector_0"]

# Find a peripheral by type (returns name or None)
name = peripheral.find("chat_box")

# Check if a specific peripheral exists
peripheral.is_present("chat_box_0")   # True/False

# Get info about a peripheral
info = peripheral.wrap("chat_box_0")  # {'name': '...', 'type': '...', 'side': '...'}
```

Peripherals are named by type with an incrementing index: `chat_box_0`, `chat_box_1`, `player_detector_0`, etc.

#### Getting Methods

```python
methods = peripheral.get_methods("chat_box_0")
for m in methods:
    print(f"  - {m}")
```

#### Calling Methods

Arguments are passed as a JSON string. Results are returned as a JSON string.

```python
# No arguments
result = peripheral.call("chat_box_0", "getName", "[]")

# With arguments
result = peripheral.call("chat_box_0", "sendMessage", '["Hello World!"]')

# Multiple arguments
result = peripheral.call("some_device", "setConfig", '[10, "label", true]')
```

---

## Examples

### Redstone Pulse

```python
import terminal

def pulse(side, duration=0.5):
    terminal.set_redstone(side, 15)
    terminal.sleep(duration)
    terminal.set_redstone(side, 0)

pulse(terminal.BACK)
terminal.println("Pulse sent!")
```

### Redstone Clock

```python
import terminal

terminal.println("Redstone clock running. Ctrl+T to stop.")
while True:
    terminal.set_redstone(terminal.BACK, 15)
    terminal.sleep(0.5)
    terminal.set_redstone(terminal.BACK, 0)
    terminal.sleep(0.5)
```

### Chat Box

```python
import terminal
import peripheral

chat = peripheral.find("chat_box")
if not chat:
    terminal.println("No chat box found! Place one next to the terminal.")
    exit()

peripheral.call(chat, "sendMessage", '["Hello from the terminal!"]')
terminal.println("Message sent!")
```

### Player Detector

```python
import terminal
import peripheral

detector = peripheral.find("player_detector")
if not detector:
    terminal.println("No player detector found!")
    exit()

result = peripheral.call(detector, "getOnlinePlayers", "[]")
terminal.println(f"Online players: {result}")
```

### Saving and Running Scripts

From the shell:
```
> edit hello.py
```
Write your script in the editor, save, then:
```
> python hello.py
```

Or create files from Python directly:
```python
import terminal

terminal.write_file("startup.py", """
import terminal
terminal.println("Computer booted!")
terminal.set_redstone(terminal.UP, 15)
""")
```

### Custom Importable Modules

Python files saved to the terminal's filesystem can be imported normally:

```python
import terminal

# Create a module
terminal.write_file("utils.py", """
def greet(name):
    return f"Hello, {name}!"

PI = 3.14159
""")
```

Then in the REPL or another script:
```python
import utils

print(utils.greet("Steve"))   # Hello, Steve!
print(utils.PI)               # 3.14159
```

This works because the OS installs a custom import hook that checks the virtual filesystem before falling back to standard imports.

### Peripheral Wrapper Class

For cleaner code when working with peripherals frequently:

```python
import peripheral

class Device:
    def __init__(self, name):
        self.name = name

    def call(self, method, args="[]"):
        return peripheral.call(self.name, method, args)

    def methods(self):
        return peripheral.get_methods(self.name)

# Usage
chat = Device("chat_box_0")
chat.call("sendMessage", '["Automated message"]')
```

### Monitor with Redstone Input

A script that enables redstone output only while the script is running:

```python
import terminal

terminal.println("Monitoring... Ctrl+T to stop")
terminal.set_redstone(terminal.BACK, 15)

count = 0
while True:
    count += 1
    terminal.set_cursor(0, 1)
    terminal.write(f"Ticks: {count}   ")
    terminal.sleep(1.0)
```

## Building from Source

### Prerequisites

- Java 21
- Rust toolchain with `wasm32-unknown-unknown` target (`rustup target add wasm32-unknown-unknown`)
- Gradle (wrapper included)

### Build

```bash
./copy-jar.sh
```

This will:
1. Compile the Rust operating systems to WASM
2. Build the mod JAR (with WASM binaries bundled inside)
3. Copy the final JAR to the project root

The WASM files are extracted from the JAR automatically when the mod loads.

### Manual Build

```bash
# Build Rust WASM
cd operating-system/rust && cargo build --release
cd operating-system/simple && cargo build --release --target wasm32-unknown-unknown

# Copy WASM to wasm-bin/
mkdir -p wasm-bin
cp operating-system/rust/target/wasm32-unknown-unknown/release/terminal_os.wasm wasm-bin/
cp operating-system/simple/target/wasm32-unknown-unknown/release/simple.wasm wasm-bin/terminal.wasm

# Build mod
./gradlew build
```

## Installation

1. Install [NeoForge](https://neoforged.net/) for Minecraft 1.21.1
2. Download the mod JAR from [Releases](https://github.com/ewitulsk/EvanModCursor/releases)
3. Place it in your `mods/` folder
4. (Optional) Install [CC: Tweaked](https://modrinth.com/mod/cc-tweaked) for peripheral support

## License

MIT
