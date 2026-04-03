# Port Changes: MC 1.21.1 (NeoForge 21.1.77) → MC 26.1 (NeoForge 26.1.0.1-beta)

## Build System

| File | Change |
|------|--------|
| `gradle-wrapper.properties` | Gradle 8.10.2 → 8.14.2 |
| `gradle.properties` | MC 1.21.1 → 26.1, NeoForge 21.1.77 → 26.1.0.1-beta, removed parchment mappings |
| `build.gradle` | MDG 2.0.42-beta → 2.0.141, Java 21 → 25, removed parchment block/repo, `data` run → `clientData`, commented out CC:Tweaked dependency (not yet available for 26.1) |

## Global Renames

### `ResourceLocation` → `Identifier` (11 files)
All usages of `net.minecraft.resources.ResourceLocation` replaced with `net.minecraft.resources.Identifier`, including `ResourceLocation.fromNamespaceAndPath()` → `Identifier.fromNamespaceAndPath()`.

**Files:** `EvansComputerMod.java`, `TerminalScreen.java`, `TerminalInputPacket.java`, `TerminalOutputPacket.java`, `OpenVisualEditorPacket.java`, `RunVisualScriptPacket.java`, `SaveVisualProgramPacket.java`, `LoadVisualProgramPacket.java`, `RequestProgramListPacket.java`, `ProgramListResponsePacket.java`, `LoadProgramResponsePacket.java`

### `javax.annotation.Nullable` → `org.jspecify.annotations.Nullable` (13 files)
JSpecify annotations are "type-use" only — some placements needed adjustment (e.g., removed `@Nullable` from `setPlacedBy` parameter where the base class doesn't annotate it).

**Files:** `TerminalBlockEntity.java`, `TerminalBlock.java`, `TerminalMenu.java`, `NetworkCableBlock.java`, `InterfaceBlock.java`, `IComputerHost.java`, `IWorldAccess.java`, `ComputerContext.java`, `ComputerModuleRegistry.java`, `ComputerRegistry.java`, `PeripheralManager.java`, `PeripheralMethodInvoker.java`, `LuaWasmTypeConverter.java`

## Registry Changes

### `ModBlockEntities.java`
- `BlockEntityType.Builder.of(...).build(null)` → `new BlockEntityType<>(...)`

### `ModBlocks.java`
- `DeferredRegister.create(Registries.BLOCK, ...)` → `DeferredRegister.createBlocks(...)`
- `DeferredRegister.create(Registries.ITEM, ...)` → `DeferredRegister.createItems(...)`
- `DeferredHolder<Block, ...>` → `DeferredBlock<...>`
- `DeferredHolder<Item, ...>` → `DeferredItem<...>`
- `BLOCKS.register("name", () -> new Block(...))` → `BLOCKS.registerBlock("name", Block::new, properties)`
- `BLOCK_ITEMS.register("name", () -> new BlockItem(...))` → `BLOCK_ITEMS.registerSimpleBlockItem("name", BLOCK)`

## NBT Serialization — `TerminalBlockEntity.java`

### Method signatures
- `saveAdditional(CompoundTag tag, HolderLookup.Provider registries)` → `saveAdditional(ValueOutput output)`
- `loadAdditional(CompoundTag tag, HolderLookup.Provider registries)` → `loadAdditional(ValueInput input)`
- Imports moved from `net.minecraft.nbt` to `net.minecraft.world.level.storage`

### UUID storage
- `tag.putUUID("computerId", uuid)` → `output.store("computerId", UUIDUtil.CODEC, uuid)`
- `tag.getUUID("computerId")` → `input.read("computerId", UUIDUtil.CODEC)`

### Byte array storage
- `tag.putByteArray("framebuffer", data)` → `output.putString("framebuffer", Base64.getEncoder().encodeToString(data))`
- `tag.getByteArray("framebuffer")` → `input.getString("framebuffer").map(Base64.getDecoder()::decode)`

### Primitive getters
- `tag.getBoolean("key")` → `input.getBooleanOr("key", defaultValue)`
- `tag.getString("key")` → `input.getStringOr("key", defaultValue)`
- `tag.contains("key")` checks replaced by `Optional`-based patterns or `*Or` defaults
- `tag.getIntArray("key")` → `input.getIntArray("key")` (now returns `Optional<int[]>`)

### Item stack UUID persistence (`TerminalBlock.java`)
- `tag.putUUID(...)` → two `tag.putLong(...)` calls (most/least significant bits)
- `tag.getUUID(...)` → reconstruct from two `tag.getLongOr(...)` calls
- `DataComponents.BLOCK_ENTITY_DATA` → `DataComponents.CUSTOM_DATA`

## Screen Rendering

### Rendering lifecycle (all screen classes)
- `render(GuiGraphics, int, int, float)` → `extractRenderState(GuiGraphicsExtractor, int, int, float)`
- `renderBg(GuiGraphics, float, int, int)` → removed (logic in `extractRenderState`)
- `renderBackground(gfx, mouseX, mouseY, partialTick)` → `extractBackground(gfx, mouseX, mouseY, partialTick)`
- `extractBackground` visibility changed from `protected` to `public`
- `GuiGraphics` class removed entirely; replaced by `GuiGraphicsExtractor`

### Drawing methods
- `gfx.drawString(font, text, x, y, color, shadow)` → `gfx.text(font, text, x, y, color, shadow)`

### Pose stack (2D transform)
- `gfx.pose()` now returns `org.joml.Matrix3x2fStack` (was `PoseStack`)
- `pushPose()` → `pushMatrix()`
- `popPose()` → `popMatrix()`
- `translate(x, y, z)` → `translate(x, y)` (2D only)
- `scale(sx, sy, sz)` → `scale(sx, sy)` (2D only)

### Font system (`TerminalScreen.java`)
- `Style.withFont(Identifier)` → `Style.withFont(new FontDescription.Resource(Identifier))`

### Container screen fields
- `imageWidth` and `imageHeight` are now `final` — removed runtime assignments in `init()`

### Files affected
- `TerminalScreen.java` — full rendering rewrite
- `VisualProgrammingScreen.java` — ~100+ rendering call updates across 14 render methods

## Input Events

### Keyboard
- `keyPressed(int keyCode, int scanCode, int modifiers)` → `keyPressed(KeyEvent event)`
  - Access via `event.key()`, `event.scancode()`, `event.modifiers()`
- `charTyped(char codePoint, int modifiers)` → `charTyped(CharacterEvent event)`
  - Access via `(char) event.codepoint()`

### Mouse
- `mouseClicked(double mouseX, double mouseY, int button)` → `mouseClicked(MouseButtonEvent event, boolean focused)`
- `mouseDragged(double mouseX, double mouseY, int button, double dragX, double dragY)` → `mouseDragged(MouseButtonEvent event, double dragX, double dragY)`
- `mouseReleased(double mouseX, double mouseY, int button)` → `mouseReleased(MouseButtonEvent event)`
  - All access via `event.x()`, `event.y()`, `event.button()`
- `mouseScrolled(double, double, double, double)` — unchanged

## Networking

### Client packet sending
- `PacketDistributor.sendToServer(...)` → `ClientPacketDistributor.sendToServer(...)`
- Import: `net.neoforged.neoforge.network.PacketDistributor` → `net.neoforged.neoforge.client.network.ClientPacketDistributor`
- **Files:** `TerminalScreen.java`, `VisualProgrammingScreen.java`

### Event bus
- `@EventBusSubscriber(modid = ..., bus = EventBusSubscriber.Bus.MOD)` → `@EventBusSubscriber(modid = ...)`
- Bus routing is now automatic based on event type
- **Files:** `ModNetwork.java`, `ClientSetup.java`

## Block Method Signatures

### `level.isClientSide` → `level.isClientSide()` (all block/entity files)
Field became private; must use accessor method. Affected every file that checked client/server side.

### `InteractionResult.sidedSuccess(boolean)` → `InteractionResult.SUCCESS`
- **File:** `TerminalBlock.java`

### `neighborChanged` signature
- Old: `(BlockState, Level, BlockPos, Block, BlockPos, boolean)`
- New: `(BlockState, Level, BlockPos, Block, Orientation, boolean)`
- **Files:** `TerminalBlock.java`, `NetworkCableBlock.java`, `InterfaceBlock.java`

### `updateShape` signature
- Old: `(BlockState, Direction, BlockState, LevelAccessor, BlockPos, BlockPos)`
- New: `(BlockState, LevelReader, ScheduledTickAccess, BlockPos, Direction, BlockPos, BlockState, RandomSource)`
- **Files:** `NetworkCableBlock.java`, `InterfaceBlock.java`

### `onRemove` → `affectNeighborsAfterRemoval`
- Old: `onRemove(BlockState, Level, BlockPos, BlockState, boolean)`
- New: `affectNeighborsAfterRemoval(BlockState, ServerLevel, BlockPos, boolean)`
- **Files:** `NetworkCableBlock.java`, `InterfaceBlock.java`

### `getCloneItemStack` signature
- Old: `(BlockState, HitResult, LevelReader, BlockPos, Player)`
- New: `(LevelReader, BlockPos, BlockState, boolean)`
- **File:** `TerminalBlock.java`

### `ChunkPos` construction
- `new ChunkPos(blockPos)` → `ChunkPos.containing(blockPos)`
- **File:** `TerminalBlockEntity.java`

### `DirectionProperty` → `EnumProperty<Direction>`
- **File:** `TerminalBlock.java`

## Command System — `WasmCommand.java`

- `source.hasPermission(2)` → new permission check system (`Commands.hasPermission(...)` with `PermissionCheck`/`Permissions`)
