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

## WASM Memory Map

The host (Java) and guest (Rust OS) communicate through fixed memory regions in the WASM linear memory:

| Address | Size | Purpose | Direction |
|---------|------|---------|-----------|
| `0x10000` (64 KiB) | Up to ~4 KiB | Input buffer — keyboard input for `on_input()` | Host → WASM |
| `0x11000` (68 KiB) | Up to ~4 KiB | Interrupt data buffer — payload for `on_interrupt()` | Host → WASM |

### WASM Exports (called by host)

| Export | Signature | Description |
|--------|-----------|-------------|
| `main()` | `() -> void` | Called once when terminal opens |
| `on_input(ptr, len)` | `(i32, i32) -> void` | Called per keyboard input |
| `on_interrupt(irq, ptr, len)` | `(i32, i32, i32) -> void` | Called to deliver an interrupt event |

### Host Functions (callable from WASM)

| Function | Signature | Description |
|----------|-----------|-------------|
| `terminal_write` | `(ptr, len) -> i32` | Write text to terminal |
| `terminal_clear` | `() -> void` | Clear screen |
| `terminal_set_cursor` | `(x, y) -> void` | Move cursor |
| `terminal_get_width` | `() -> i32` | Returns 80 |
| `terminal_get_height` | `() -> i32` | Returns 24 |
| `sleep_ms` | `(ms) -> void` | Sleep (Rust side chunks for interrupt delivery) |
| `redstone_set_output` | `(side, power) -> i32` | Set redstone output (0–15) |
| `redstone_get_input` | `(side) -> i32` | Read redstone input (0–15) |
| `redstone_get_all_input` | `(buf_ptr) -> i32` | Read all 6 input sides into buffer |
| `interrupt_poll` | `(buf_ptr, buf_len) -> i32` | Poll next pending interrupt (returns IRQ or -1) |
| `interrupt_poll_len` | `() -> i32` | Get payload length of last polled interrupt |
| `file_read/write/delete/exists/size/list` | various | Virtual filesystem operations |
| `peripheral_list/get_methods/call` | various | CC:Tweaked peripheral access |

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

Terminals can read and write redstone signals (0-15) on all six sides. Sides are relative to the terminal's facing direction.

```python
import terminal

# Side constants
terminal.DOWN    # 0
terminal.UP      # 1
terminal.FRONT   # 2 (screen side)
terminal.BACK    # 3
terminal.LEFT    # 4
terminal.RIGHT   # 5

# Set output power level (0-15)
terminal.set_redstone(terminal.BACK, 15)   # Full power behind terminal
terminal.set_redstone(terminal.BACK, 0)    # Off

# Read input power level (0-15)
power = terminal.get_redstone(terminal.BACK)
levels = terminal.get_all_redstone()   # [DOWN, UP, FRONT, BACK, LEFT, RIGHT]
```

#### Interrupts

Programs can register interrupt handlers that fire when hardware events occur. Interrupts are cooperative — they are delivered during `sleep()`, `check_interrupts()`, or between commands.

```python
import terminal

# Register a handler for redstone changes
def on_redstone(data):
    terminal.println(f"Redstone changed! Sides: {data['sides']}")

terminal.on_interrupt(terminal.IRQ_REDSTONE, on_redstone)

# Register a handler for keypresses during execution
def on_key(data):
    terminal.println(f"Key pressed: {data['key']}")

terminal.on_interrupt(terminal.IRQ_KEYBOARD, on_key)

# In a loop, call check_interrupts() or sleep() to deliver events
while True:
    terminal.sleep(0.1)

# Clear a handler when done
terminal.clear_interrupt(terminal.IRQ_REDSTONE)
```

| IRQ Constant | Value | Event |
|-------------|-------|-------|
| `terminal.IRQ_KEYBOARD` | 1 | Key pressed during execution |
| `terminal.IRQ_REDSTONE` | 2 | Redstone input level changed |

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

### Redstone Input Monitor

Read redstone input and react to changes using interrupts:

```python
import terminal

def on_redstone(data):
    sides = data['sides']
    terminal.set_cursor(0, 2)
    terminal.write(f"Input levels: {sides}   ")
    # Mirror back input to front output
    terminal.set_redstone(terminal.FRONT, sides[3])  # BACK input -> FRONT output

terminal.on_interrupt(terminal.IRQ_REDSTONE, on_redstone)

terminal.println("Redstone monitor running. Ctrl+T to stop.")
terminal.println("Apply redstone to any side to see levels.")

while True:
    terminal.sleep(1.0)
```

### Keyboard Event Logger

Log keypresses during execution:

```python
import terminal

keys = []

def on_key(data):
    keys.append(data['key'])
    terminal.set_cursor(0, 2)
    terminal.write(f"Last key: {repr(data['key'])}  Total: {len(keys)}   ")

terminal.on_interrupt(terminal.IRQ_KEYBOARD, on_key)

terminal.println("Press keys while running. Ctrl+T to stop.")
while True:
    terminal.sleep(0.1)
```

---

## Mod Integration API

EvansComputerMod is designed for other mods to extend. You can expose Java methods to Python with a few annotations — no WASM knowledge required. You can also embed a full computer into your own blocks, entities, or items.

### Quick Start: Exposing Functions to Python

**1. Add EvansComputerMod as a dependency** in your `build.gradle`:

```groovy
dependencies {
    implementation 'com.example:evanscomputermod:1.0.0'
}
```

**2. Create an annotated module class:**

```java
import com.example.evanscomputermod.api.*;

@ComputerModule(value = "golem", description = "Golem control API")
public class GolemAPI {

    @ComputerFunction(description = "Summon a golem of the given type")
    public boolean summon(ComputerContext ctx, String type) {
        // ctx gives you computerId, level, position, server
        // ... your mod logic here ...
        return true;
    }

    @ComputerFunction(description = "Get the health of a golem")
    public int getHealth(String golemId) {
        // ComputerContext is optional — omit it if you don't need world access
        return 20;
    }

    @ComputerFunction(description = "Detonate a golem", mainThread = true)
    public void detonate(ComputerContext ctx, String golemId) {
        // mainThread = true ensures this runs on the server tick thread
        // (required for any operation that modifies the world)
    }
}
```

**3. Register during the setup event:**

```java
import com.example.evanscomputermod.api.ComputerModuleRegistry;
import com.example.evanscomputermod.api.RegisterComputerModulesEvent;
import net.neoforged.bus.api.SubscribeEvent;

public class MyMod {
    @SubscribeEvent
    public void onRegisterModules(RegisterComputerModulesEvent event) {
        ComputerModuleRegistry.register(new GolemAPI());
    }
}
```

**4. That's it.** Python users can now do:

```python
import golem

golem.summon("iron")          # calls GolemAPI.summon()
health = golem.get_health("golem_1")  # calls GolemAPI.getHealth()
golem.detonate("golem_1")     # runs on main thread
```

Visual programming blocks are also auto-generated — one block per function, with typed input/output ports.

### Annotation Reference

#### `@ComputerModule(value, description)`

| Attribute | Type | Required | Description |
|-----------|------|----------|-------------|
| `value` | `String` | Yes | Python module name (what users `import`) |
| `description` | `String` | No | Description for visual block category |

#### `@ComputerFunction(value, description, mainThread)`

| Attribute | Type | Default | Description |
|-----------|------|---------|-------------|
| `value` | `String` | `""` | Override Python function name (default: camelCase converted to snake_case) |
| `description` | `String` | `""` | Description for visual blocks and help text |
| `mainThread` | `boolean` | `false` | Execute on the server main thread (required for world modifications) |

#### `ComputerContext`

Injected as the first parameter of your method if present. Not visible to Python callers.

| Field | Type | Description |
|-------|------|-------------|
| `getComputerId()` | `UUID` | Unique persistent ID of the computer |
| `getPosition()` | `BlockPos` | World position (null if headless) |
| `getLevel()` | `Level` | Minecraft level/world (null if headless) |
| `getServer()` | `MinecraftServer` | Server instance |

#### Supported Types

| Java Type | Python Type | Binary Tag |
|-----------|-------------|------------|
| `String` | `str` | `0x01` |
| `int` / `Integer` | `int` | `0x02` |
| `long` / `Long` | `int` | `0x03` |
| `float` / `Float` / `double` / `Double` | `float` | `0x04` |
| `boolean` / `Boolean` | `bool` | `0x05` |
| `void` | `None` | `0x00` |

---

### Embedding a Computer in Your Own Block/Entity/Item

The computer runtime is fully decoupled from the terminal block. You can embed a computer into any context by implementing `IComputerHost`.

```java
import com.example.evanscomputermod.api.*;
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.ComputerRegistry;

public class DroneEntity extends Entity implements IComputerHost {
    private final UUID computerId = UUID.randomUUID();
    private ComputerInstance computer;

    // Required: identity and server access
    @Override public UUID getComputerId() { return computerId; }
    @Override public MinecraftServer getServer() { return level().getServer(); }
    @Override public void markDirty() { /* entity state changed */ }
    @Override public void syncToClients() { /* send updates to tracking players */ }

    // Optional capabilities — return null to disable
    @Override public ITerminalOutput getTerminalOutput() { return null; }  // headless
    @Override public IRedstoneProvider getRedstoneProvider() { return null; }
    @Override public IWorldAccess getWorldAccess() {
        return new IWorldAccess() {
            public Level getLevel() { return DroneEntity.this.level(); }
            public BlockPos getBlockPos() { return DroneEntity.this.blockPosition(); }
        };
    }
    @Override public IVisualProgramming getVisualProgramming() { return null; }

    public void startComputer() {
        computer = new ComputerInstance(this);
        computer.loadModule("terminal_os");
        computer.executeMain();
        computer.startWorkerThread();
        ComputerRegistry.register(this);
    }

    @Override public void remove(RemovalReason reason) {
        if (computer != null) { computer.close(); }
        ComputerRegistry.unregister(computerId);
        super.remove(reason);
    }
}
```

#### `IComputerHost` Capabilities

| Method | Returns | Required | Purpose |
|--------|---------|----------|---------|
| `getComputerId()` | `UUID` | Yes | Persistent identity for file storage |
| `getServer()` | `MinecraftServer` | Yes | Main thread scheduling |
| `markDirty()` | `void` | Yes | Signal state needs persistence |
| `syncToClients()` | `void` | Yes | Push display updates to clients |
| `getTerminalOutput()` | `ITerminalOutput` | No (null = headless) | 80x24 character display |
| `getRedstoneProvider()` | `IRedstoneProvider` | No (null = no redstone) | Redstone I/O |
| `getWorldAccess()` | `IWorldAccess` | No (null = no peripherals) | Position for peripheral scanning |
| `getVisualProgramming()` | `IVisualProgramming` | No (null = disabled) | Visual editor support |

---

### How It Works: Technical Architecture

Understanding the internals is not required to use the API, but this section explains how annotations become Python functions.

#### The WASM Recompilation Problem

The computer runs a Rust OS compiled to WebAssembly. Host functions (Java methods callable from WASM) must be declared as `extern "C"` in Rust **at compile time**. Third-party mods cannot modify the WASM binary.

The solution: a single generic bridge function `module_call` is compiled into the WASM binary once. All third-party module calls route through it. The bridge uses a binary protocol instead of JSON for efficiency.

#### The Full Pipeline

```
Python: golem.summon("iron")
  |
  | (1) Python bootstrap auto-generated this function at startup.
  |     It calls: _modules.call("golem", "summon", "iron")
  v
Rust (python.rs): serialize_args_binary()
  |
  | (2) Converts Python objects to binary: [0x01 arg_count] [0x01 tag=string] [0x04 len] [iron]
  |     No JSON escaping, no string building — just type tag + raw bytes.
  v
Rust (modules.rs): module_call() extern "C"
  |
  | (3) Writes module name, method name, and binary args into WASM linear memory.
  |     Calls the host function (crosses WASM→Java boundary via Wasmtime).
  v
Java (ComputerInstance): hostModuleCall()
  |
  | (4) Reads module/method names as strings from WASM memory.
  |     Reads args as raw bytes (no string conversion).
  |     Delegates to ModuleMethodInvoker.
  v
Java (ModuleMethodInvoker): parseBinaryArgs()
  |
  | (5) Reads type tags from binary buffer.
  |     Constructs typed Java objects directly (Integer, String, Boolean, etc.)
  |     Uses ParameterInfo from @ComputerFunction annotation for type coercion.
  |     Injects ComputerContext if method expects it.
  v
Java: GolemAPI.summon(ctx, "iron")
  |
  | (6) Your mod code runs. Returns a boolean.
  v
Java (ModuleMethodInvoker): serializeResult()
  |
  | (7) Writes: [0x00 status=ok] [0x05 tag=bool] [0x01 value=true]
  |     3 bytes total. No JSON object construction.
  v
WASM memory → Rust: parse_binary_result()
  |
  | (8) Reads status byte, type tag, payload.
  |     Returns BinaryValue::Bool(true). No string parsing.
  v
Rust (python.rs): binary_value_to_pyobj()
  |
  | (9) Converts BinaryValue to Python bool directly.
  v
Python: True
```

#### Binary Wire Format

Arguments and results use a compact type-tagged binary encoding:

```
Arguments: [u8 arg_count] ([u8 type_tag] [payload])*
Result:    [u8 status] [u8 type_tag] [payload]

Type tags:
  0x00 = null
  0x01 = string:  [u32 LE length] [UTF-8 bytes]
  0x02 = i32:     [4 bytes LE]
  0x03 = i64:     [8 bytes LE]
  0x04 = f64:     [8 bytes LE]
  0x05 = bool:    [1 byte, 0 or 1]

Status: 0x00 = success, 0x01 = error (followed by string message)
```

Example: `golem.summon("iron")` produces 10 bytes of args (`01 01 04000000 69726F6E`) and 3 bytes of result (`00 05 01`). Compare this to the equivalent JSON which would be `["iron"]` (8 bytes) and `{"ok":true,"result":true}` (24 bytes) — plus the overhead of parsing both.

#### Auto-Generated Python Modules

At interpreter startup, `python_bootstrap.py` calls `_modules.get_metadata()` which returns JSON metadata describing all registered modules and their functions. The bootstrap creates a Python `ModuleType` for each module and populates it with wrapper functions that delegate to `_modules.call()`. This happens once, before any user code runs.

#### Auto-Generated Visual Programming Blocks

`VisualBlockRegistry.generateBlocksFromModules()` iterates `ComputerModuleRegistry` at load time and creates visual block definitions for each registered function. Each block gets typed input ports (from `ParameterInfo`), flow ports, and a code template that generates the correct Python call. A deterministic color is assigned based on the module name.

---

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
