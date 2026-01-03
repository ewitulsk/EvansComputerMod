# WASM Integration

This mod integrates [wasmtime-java](https://github.com/kawamuray/wasmtime-java) to allow executing WebAssembly functions from within Minecraft.

## Overview

- **Dependency**: `io.github.kawamuray.wasmtime:wasmtime-java:0.19.0`
- **WASM Directory**: `wasm-bin/` (created automatically at mod startup)
- **Command**: `/wasm`

## Usage

### Command Syntax

```
/wasm <file> <function> [parameters...]
/wasm list
```

### Examples

```
/wasm add add 5 3
/wasm calculator multiply 10 20
/wasm math.wasm compute 1.5 2.5
/wasm list
```

The `.wasm` extension is optional when specifying the file name.

### Permissions

Requires operator level 2 (same as `/tp`, `/give`, etc.)

## Supported Parameter Types

| WASM Type | Input Format | Example |
|-----------|--------------|---------|
| `i32`     | Integer      | `42`    |
| `i64`     | Long         | `9999999999` |
| `f32`     | Float        | `3.14`  |
| `f64`     | Double       | `3.14159265359` |

Parameters are automatically parsed based on the function's type signature in the WASM module.

## Architecture

### WasmManager

Located at `com.example.customworld.wasm.WasmManager`

Responsibilities:
- Creates `wasm-bin/` directory on mod initialization
- Loads WASM modules from disk
- Executes exported functions with type-safe parameter parsing
- Lists available `.wasm` files

### WasmCommand

Located at `com.example.customworld.command.WasmCommand`

Responsibilities:
- Registers `/wasm` command with Brigadier
- Provides tab-completion for available WASM files
- Handles parameter parsing (supports quoted strings)
- Displays results with colored chat output

## Creating WASM Modules

Any language that compiles to WASM can be used. The module must export the function you want to call.

### Rust Example

```rust
#[no_mangle]
pub extern "C" fn add(a: i32, b: i32) -> i32 {
    a + b
}
```

Compile with:
```bash
rustc --target wasm32-unknown-unknown -O --crate-type=cdylib add.rs -o add.wasm
```

### C Example

```c
__attribute__((export_name("multiply")))
int multiply(int a, int b) {
    return a * b;
}
```

Compile with:
```bash
clang --target=wasm32 -nostdlib -Wl,--no-entry -Wl,--export-all -o multiply.wasm multiply.c
```

## File Structure

```
game-directory/
├── wasm-bin/           # Place .wasm files here
│   ├── add.wasm
│   ├── calculator.wasm
│   └── ...
├── mods/
│   └── customworld-1.0.0.jar
└── ...
```

## Error Handling

The command provides descriptive error messages for:
- File not found
- Function not found in module
- Parameter count mismatch
- Parameter type parsing failures
- WASM runtime errors
