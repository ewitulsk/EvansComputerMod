# Rendering Pipeline & Display Block Implementation Plan

## Overview

Build a **pixel framebuffer rendering pipeline** alongside the existing text terminal system. This adds:
1. A new `Framebuffer` abstraction (server-side pixel buffer with dirty-tile diffing)
2. New WASM host functions for pixel drawing (`fb_write`, `fb_flush`, etc.)
3. A new **Display Block** (like CC:Tweaked monitors — multi-block, configurable size)
4. Client-side `DynamicTexture` rendering on the display block face
5. A new network protocol for efficient tile-based pixel sync
6. Simulator support for both the terminal UI and the new pixel display

---

## Phase 1: Framebuffer Core (Server-Side)

### 1.1 — `Framebuffer.java` (new file)
**Path:** `src/main/java/com/example/evanscomputermod/computer/Framebuffer.java`

Server-side RGBA pixel buffer with dirty-tile tracking.

```
Framebuffer(int width, int height, int tileSize)
  - byte[] pixels          — RGBA, length = width * height * 4
  - byte[] previousPixels  — snapshot from last flush (for diffing)
  - boolean[] dirtyTiles   — one flag per tile
  - int tileSize           — e.g., 16 (16x16 pixel tiles)
  - int tilesX, tilesY     — grid dimensions

Methods:
  - setPixel(x, y, r, g, b, a)
  - fillRect(x, y, w, h, r, g, b, a)
  - writeRegion(x, y, w, h, byte[] rgbaData)   — bulk write from WASM memory
  - clear(r, g, b, a)
  - markDirty(x, y)                             — marks the tile containing (x,y)
  - flush() → List<DirtyTile>                   — diffs against previousPixels, returns changed tiles, updates snapshot
  - getWidth(), getHeight()
```

`DirtyTile` record: `(tileX, tileY, byte[] pixelData)` — the raw RGBA pixels for that tile region.

### 1.2 — `IFramebufferHost.java` (new interface)
**Path:** `src/main/java/com/example/evanscomputermod/api/IFramebufferHost.java`

```java
public interface IFramebufferHost {
    Framebuffer getFramebuffer();
    int getDisplayWidth();    // in pixels
    int getDisplayHeight();   // in pixels
}
```

This is what the Display block entity implements. The Computer block does NOT implement this — it keeps using `ITerminalOutput`.

### 1.3 — New WASM Host Functions

Add to `ComputerInstance.java`'s linker registration:

| Host Function | Signature | Description |
|---|---|---|
| `fb_get_width` | `() → i32` | Returns framebuffer width in pixels |
| `fb_get_height` | `() → i32` | Returns framebuffer height in pixels |
| `fb_set_pixel` | `(x: i32, y: i32, r: i32, g: i32, b: i32, a: i32)` | Set one pixel |
| `fb_fill_rect` | `(x: i32, y: i32, w: i32, h: i32, r: i32, g: i32, b: i32, a: i32)` | Fill rectangle |
| `fb_write_region` | `(x: i32, y: i32, w: i32, h: i32, ptr: i32, len: i32)` | Bulk write RGBA from WASM memory |
| `fb_clear` | `(r: i32, g: i32, b: i32, a: i32)` | Clear entire buffer |
| `fb_flush` | `() → i32` | Flush dirty tiles to clients, returns number of tiles sent |
| `fb_blit_text` | `(x: i32, y: i32, ptr: i32, len: i32, fg_r: i32, fg_g: i32, fg_b: i32, bg_r: i32, bg_g: i32, bg_b: i32)` | Rasterize text string into framebuffer at pixel position |

These functions check if the host implements `IFramebufferHost`. If not (e.g., a terminal computer), they return -1 / no-op.

### 1.4 — Rust OS Bindings

**Path:** `operating-system/rust/src/lib.rs` (add `fb` module)
**Path:** `operating-system/rust/src/fb.rs` (new file)

```rust
pub mod fb {
    extern "C" {
        fn fb_get_width() -> i32;
        fn fb_get_height() -> i32;
        fn fb_set_pixel(x: i32, y: i32, r: i32, g: i32, b: i32, a: i32);
        fn fb_fill_rect(x: i32, y: i32, w: i32, h: i32, r: i32, g: i32, b: i32, a: i32);
        fn fb_write_region(x: i32, y: i32, w: i32, h: i32, ptr: *const u8, len: i32);
        fn fb_clear(r: i32, g: i32, b: i32, a: i32);
        fn fb_flush() -> i32;
        fn fb_blit_text(x: i32, y: i32, ptr: *const u8, len: i32,
                        fg_r: i32, fg_g: i32, fg_b: i32,
                        bg_r: i32, bg_g: i32, bg_b: i32);
    }

    // Safe wrappers...
}
```

Also expose these to RustPython via a `display` Python module so users can write:
```python
import display
display.clear(0, 0, 0)
display.fill_rect(10, 10, 100, 50, 255, 0, 0)
display.text(10, 70, "Hello", fg=(255,255,255), bg=(0,0,0))
display.flush()
```

---

## Phase 2: Network Protocol

### 2.1 — `FramebufferUpdatePacket.java` (new file)
**Path:** `src/main/java/com/example/evanscomputermod/network/FramebufferUpdatePacket.java`

Sends dirty tiles from server to client. Binary format:

```
BlockPos pos
int fullWidth, fullHeight        — so client knows framebuffer dimensions
int tileSize                     — tile dimension (e.g., 16)
int tileCount                    — number of dirty tiles in this packet
for each tile:
    short tileX, tileY           — tile grid coordinates
    byte[] pixelData             — raw RGBA, length = tileSize * tileSize * 4
                                   (compressed with deflate for network efficiency)
```

Register in `ModNetwork.java`. Handle on client by patching the corresponding `NativeImage` region.

### 2.2 — `FramebufferFullPacket.java` (new file)
**Path:** `src/main/java/com/example/evanscomputermod/network/FramebufferFullPacket.java`

Sent when a player first starts tracking the chunk containing a display block (or opens it). Contains the entire framebuffer as compressed RGBA.

### 2.3 — Flush Integration

On `fb_flush()` (called from WASM):
1. `Framebuffer.flush()` returns list of dirty tiles
2. For each tracking player, send `FramebufferUpdatePacket` containing the dirty tiles
3. If tile count exceeds threshold (e.g., >75% of tiles dirty), send `FramebufferFullPacket` instead

Alternative: flush on server tick instead of on WASM call. The server tick handler checks if the framebuffer has pending dirty tiles and sends them. This decouples WASM write rate from network rate. **Recommended: both** — `fb_flush()` triggers an immediate send, and a tick-based auto-flush catches any accumulated changes from CEF in the future.

---

## Phase 3: Display Block (Server-Side)

### 3.1 — `DisplayBlock.java` (new file)
**Path:** `src/main/java/com/example/evanscomputermod/block/DisplayBlock.java`

A block that acts as a pixel display surface. Like CC:Tweaked monitors:

- **Directional** — has a FACING property (which face shows the display)
- **Multi-block** — can be placed in a grid to form larger displays
  - When placed adjacent to another DisplayBlock on the same plane/facing, they merge into one logical display
  - The **origin block** (bottom-left) holds the `DisplayBlockEntity` with the framebuffer
  - Other blocks in the group are "secondary" — they store a reference to the origin
- **Configurable resolution** — default resolution per block: 128x128 pixels. A 3x2 display = 384x256 pixels
- **No GUI** — the display is rendered directly on the block face (in-world), not in a screen

Properties:
```java
FACING: DirectionProperty (NORTH, SOUTH, EAST, WEST, UP, DOWN)
```

### 3.2 — `DisplayBlockEntity.java` (new file)
**Path:** `src/main/java/com/example/evanscomputermod/block/DisplayBlockEntity.java`

Implements `IFramebufferHost` only (NOT `IComputerHost` — it has no computer).

Key fields:
```java
Framebuffer framebuffer;       // created based on multi-block dimensions
int blocksWide, blocksTall;    // how many blocks in the display grid
BlockPos originPos;            // origin block position (bottom-left)
boolean isOrigin;              // true for origin block, false for secondaries
@Nullable BlockPos controllerPos;  // the Terminal block controlling this display
```

Multi-block merging logic:
- When a DisplayBlock is placed, scan adjacent blocks on the same plane/facing
- If neighbors are DisplayBlocks with same facing, merge into one group
- Resize framebuffer to match new dimensions
- When a block is broken, split the group (or shrink)

The display has no computer and no terminal. It is a passive pixel surface controlled by an adjacent Terminal block whose WASM programs use `fb_*` functions.

### 3.3 — Block/Entity Registration

Add to `ModBlocks.java`, `ModBlockEntities.java`, `ModCreativeTabs.java`:
- `DISPLAY_BLOCK` registration
- `DISPLAY_BLOCK_ENTITY` registration
- Add to creative tab

### 3.4 — Computer Attachment (Option B: External Connection)

The display block is **passive** — it has no computer of its own. A separate Terminal block "connects" to it, and programs running on that computer can draw to the attached display via `fb_*` functions.

**Discovery mechanism:**
- When a `TerminalBlockEntity` initializes (or on neighbor change), it scans adjacent blocks for `DisplayBlockEntity` instances
- If found, the terminal registers as the display's controller: `displayBlockEntity.setController(terminalBlockEntity)`
- The display stores a weak reference (BlockPos) to its controller — not a direct reference, to survive chunk load/unload
- A display can only have **one** controller at a time. A computer can have **one** attached display at a time.

**How `fb_*` host functions resolve the framebuffer:**
- `ComputerInstance` gains a nullable `IFramebufferHost attachedDisplay` field
- When the terminal discovers an adjacent display, it sets `computer.setAttachedDisplay(displayBlockEntity)`
- `fb_get_width()`, `fb_set_pixel()`, etc. check `attachedDisplay`. If null, they return -1 (no display attached)
- `fb_flush()` calls `attachedDisplay.getFramebuffer().flush()` and sends packets to tracking players of the *display's* chunk

**`IComputerHost` changes:**
- Add `@Nullable IFramebufferHost getAttachedDisplay()` to the `IComputerHost` interface
- `TerminalBlockEntity` implements this by scanning for adjacent `DisplayBlockEntity`
- `DisplayBlockEntity` does NOT implement `IComputerHost` — it only implements `IFramebufferHost`

**Connection feedback:**
- When a display is connected/disconnected, the computer receives an interrupt (new IRQ, e.g., `IRQ_DISPLAY = 3`) so WASM programs can react
- The display block could show a visual indicator (e.g., different texture overlay or particle) when connected vs disconnected

**NBT persistence:**
- The display saves its `originPos`, `blocksWide`, `blocksTall`, and controller BlockPos
- On world load, the terminal re-discovers adjacent displays during its first tick

---

## Phase 4: Client-Side Rendering

### 4.1 — `DisplayBlockEntityRenderer.java` (new file)
**Path:** `src/main/java/com/example/evanscomputermod/client/DisplayBlockEntityRenderer.java`

A `BlockEntityRenderer<DisplayBlockEntity>` that renders the framebuffer as a textured quad on the display face.

Key mechanics:
- Maintains a `NativeImage` (same size as framebuffer) and a `DynamicTexture`
- When `FramebufferUpdatePacket` arrives, patches the `NativeImage` regions and calls `DynamicTexture.upload()`
- In `render()`, binds the dynamic texture and draws a quad on the block's front face
- For multi-block displays, only the origin block renders — it draws one large quad spanning all blocks in the group
- Quad position is calculated from `blocksWide`, `blocksTall`, and `FACING`

### 4.2 — `ClientDisplayManager.java` (new file)
**Path:** `src/main/java/com/example/evanscomputermod/client/ClientDisplayManager.java`

Manages `NativeImage` + `DynamicTexture` instances per display block entity.

```java
Map<BlockPos, DisplayClientState> displays;

class DisplayClientState {
    NativeImage image;
    DynamicTexture texture;
    ResourceLocation textureId;
    int width, height;
}
```

- Created when a display comes into view (chunk load / full packet)
- Destroyed when chunk unloads
- `applyTileUpdate(tileX, tileY, byte[] rgba)` — patches NativeImage, marks texture dirty
- `applyFullUpdate(byte[] rgba)` — replaces entire NativeImage

### 4.3 — Client Registration

In `ClientSetup.java`:
- Register `DisplayBlockEntityRenderer` with the block entity renderer registry
- Initialize `ClientDisplayManager`

---

## Phase 5: Display Block Assets

### **USER ACTION REQUIRED**

You need to provide the following assets:

- [ ] `src/main/resources/assets/evanscomputermod/textures/block/display_front.png` — front face texture (border/bezel, the framebuffer renders over the center)
- [ ] `src/main/resources/assets/evanscomputermod/textures/block/display_side.png` — side faces
- [ ] `src/main/resources/assets/evanscomputermod/textures/block/display_back.png` — back face

I will create the following automatically:
- `blockstates/display_block.json`
- `models/block/display_block.json`
- `models/item/display_block.json`
- Language entries in `lang/en_us.json`
- Loot table / recipe (if desired)

---

## Phase 6: Simulator Updates

### 6.1 — Framebuffer Host Functions in Simulator

**Path:** `simulator/src/host/framebuffer.rs` (new file)

Implement all `fb_*` host functions in the simulator. The simulator maintains its own `Framebuffer` struct (Rust equivalent of the Java one):

```rust
pub struct SimFramebuffer {
    width: usize,
    height: usize,
    pixels: Vec<u8>,        // RGBA
    dirty: bool,
}
```

Register in `simulator/src/host/mod.rs`.

### 6.2 — Display Rendering in Simulator

**Path:** `simulator/src/display_renderer.rs` (new file)

Two rendering modes for the simulator:

#### Mode A: Terminal UI Simulation (default)
The existing crossterm-based terminal rendering for the 80x24 text buffer. No changes needed — this is the current behavior.

#### Mode B: Display Simulation (`--display WxH`)
New CLI flag:
```
--display <WxH>    Simulate a pixel display (e.g., --display 256x192)
```

When `--display` is specified:
- The simulator operates in framebuffer mode instead of terminal mode
- On `fb_flush()`, render the framebuffer to the terminal using **half-block characters** (`▀▄█` ) with ANSI 24-bit color
  - Each terminal cell represents 2 vertical pixels (top half = upper pixel, bottom half = lower pixel)
  - A 256x192 display requires 256 columns x 96 rows of terminal space
  - If the terminal is smaller, scale down (nearest-neighbor) to fit
- Alternatively, output to a **PPM/PNG file** per frame (`--display-output frames/`) for debugging
- The `fb_blit_text` function renders text using a built-in 8x16 bitmap font (embedded in the simulator binary)

#### Mode C: Both Terminal + Display (`--display WxH --split`)
Show terminal output on the left, pixel display on the right (or top/bottom split). Uses crossterm's column positioning to lay out both side by side:

```
┌─ Terminal (80x24) ──────────┐  ┌─ Display (128x96) ─────────┐
│ / > python demo.py          │  │ ████████████████████████████│
│ Running: demo.py            │  │ ██  Hello World!  ██████████│
│ Drawing...                  │  │ ████████████████████████████│
│                             │  │ ██████████░░░░░░████████████│
└─────────────────────────────┘  └─────────────────────────────┘
```

### 6.3 — Custom Display Sizes

The `--width` and `--height` flags already control terminal dimensions. Add `--display` for framebuffer dimensions. They're independent:

```bash
# Text terminal only (current behavior)
cargo run -- --wasm ../wasm-bin/terminal_os.wasm --width 80 --height 24

# Pixel display only
cargo run -- --wasm ../wasm-bin/display_demo.wasm --display 256x192

# Both terminal and display
cargo run -- --wasm ../wasm-bin/terminal_os.wasm --width 80 --height 24 --display 256x192 --split
```

### 6.4 — Simulator `HostState` Changes

Add framebuffer to `HostState` in `simulator/src/wasm_host.rs`:

```rust
pub struct HostState {
    // existing fields...
    pub framebuffer: Option<SimFramebuffer>,  // None if no --display flag
}
```

---

## Phase 7: Integration & Wiring

### 7.1 — `ComputerInstance` Updates

The computer instance gets a nullable attached display reference:

```java
@Nullable private IFramebufferHost attachedDisplay;

public void setAttachedDisplay(@Nullable IFramebufferHost display) {
    this.attachedDisplay = display;
    // Register fb_* host functions in the Wasmtime linker (always registered,
    // but they return -1 when attachedDisplay is null)
}
```

The fb_* host functions are always registered in the linker (so WASM modules don't trap on missing imports), but they check `attachedDisplay != null` before operating. If null, they return -1.

`TerminalBlockEntity` scans for adjacent `DisplayBlockEntity` on init and neighbor change:
```java
// In TerminalBlockEntity.onNeighborChanged() or initializeWasm():
for (Direction dir : Direction.values()) {
    BlockEntity neighbor = level.getBlockEntity(worldPosition.relative(dir));
    if (neighbor instanceof DisplayBlockEntity display && display.getController() == null) {
        display.setController(this.worldPosition);
        if (computer != null) {
            computer.setAttachedDisplay(display);
        }
        break;  // one display per computer
    }
}
```

When `fb_flush()` is called from WASM:
1. Call `attachedDisplay.getFramebuffer().flush()` to get dirty tiles
2. Package into `FramebufferUpdatePacket`
3. Send to all players tracking the *display's* chunk (not the computer's)

### 7.2 — Tick-Based Auto-Flush

In `DisplayBlockEntity.serverTick()`:
- Check if framebuffer has any dirty tiles (e.g., from CEF in the future)
- If yes, flush and send packets
- Rate-limit to e.g., every 2 ticks (10 FPS) to avoid flooding

### 7.3 — Player Tracking

When a player starts tracking the chunk containing a display:
- Send `FramebufferFullPacket` with the current framebuffer state
- This ensures players who walk into range see the current display

---

## Summary: Files to Create/Modify

### New Java Files
| File | Description |
|------|-------------|
| `computer/Framebuffer.java` | Pixel buffer with dirty-tile tracking |
| `api/IFramebufferHost.java` | Interface for blocks with pixel displays |
| `block/DisplayBlock.java` | The display block (multi-block capable) |
| `block/DisplayBlockEntity.java` | Display block entity (passive framebuffer host, no computer) |
| `network/FramebufferUpdatePacket.java` | Dirty-tile sync packet |
| `network/FramebufferFullPacket.java` | Full framebuffer sync packet |
| `client/DisplayBlockEntityRenderer.java` | In-world display renderer |
| `client/ClientDisplayManager.java` | Client texture management |

### Modified Java Files
| File | Change |
|------|--------|
| `computer/ComputerInstance.java` | Add fb_* host function registration, `attachedDisplay` field |
| `api/IComputerHost.java` | Add `getAttachedDisplay()` method |
| `block/TerminalBlockEntity.java` | Scan for adjacent DisplayBlock, implement `getAttachedDisplay()` |
| `block/ModBlocks.java` | Register display block |
| `block/ModBlockEntities.java` | Register display block entity |
| `block/ModCreativeTabs.java` | Add display block to creative tab |
| `network/ModNetwork.java` | Register new packets |
| `client/ClientSetup.java` | Register display renderer |

### New Rust Files (OS)
| File | Description |
|------|-------------|
| `operating-system/rust/src/fb.rs` | Framebuffer extern bindings + safe wrappers |
| `operating-system/rust/src/python.rs` (modify) | Add `display` Python module |

### New Simulator Files
| File | Description |
|------|-------------|
| `simulator/src/host/framebuffer.rs` | fb_* host function implementations |
| `simulator/src/display_renderer.rs` | Half-block pixel rendering to terminal |

### Modified Simulator Files
| File | Change |
|------|--------|
| `simulator/src/main.rs` | Add `--display`, `--split` CLI flags |
| `simulator/src/wasm_host.rs` | Add framebuffer to HostState |
| `simulator/src/host/mod.rs` | Register framebuffer host functions |

### New Asset Files (created by code)
| File | Description |
|------|-------------|
| `blockstates/display_block.json` | Block state definition |
| `models/block/display_block.json` | Block model |
| `models/item/display_block.json` | Item model |
| `lang/en_us.json` (modify) | Add display block name |

### Asset Files (USER MUST PROVIDE)
| File | Description |
|------|-------------|
| `textures/block/display_front.png` | Front face (bezel) |
| `textures/block/display_side.png` | Side faces |
| `textures/block/display_back.png` | Back face |

---

## Implementation Order

1. **Phase 1** — Framebuffer core + WASM host functions (can test with unit tests)
2. **Phase 3** — Display block + block entity (needs phase 1)
3. **Phase 2** — Network packets (needs phase 1 + 3)
4. **Phase 4** — Client rendering (needs phase 2 + 3)
5. **Phase 5** — Assets (needs phase 3, **blocked on user providing textures**)
6. **Phase 1.4** — Rust OS bindings + Python module (needs phase 1)
7. **Phase 6** — Simulator updates (independent, can be done in parallel with phases 2-4)
8. **Phase 7** — Integration wiring (final, ties everything together)
