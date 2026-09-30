package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {

import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.ScreenBlock;
import com.example.evanscomputermod.block.TerminalBlock;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.overlay.ItemOverlayPacket;
import com.example.evanscomputermod.computer.overlay.ItemOverlays;
import com.example.evanscomputermod.computer.overlay.ScreenTouch;
import com.example.evanscomputermod.computer.peripheral.PeripheralValues;
import com.example.evanscomputermod.item.ModItems;
import com.example.evanscomputermod.module.ModuleBays;
import com.example.evanscomputermod.network.MouseInputPacket;
import com.example.evanscomputermod.sensor.SensorContent;
import com.example.evanscomputermod.sensor.wire.IWireHost;
import com.example.evanscomputermod.sensor.wire.SensorWireItem;
import com.example.evanscomputermod.storage.StorageContent;
import com.example.evanscomputermod.storage.core.CellContents;
import com.example.evanscomputermod.storage.core.CellTier;
import com.example.evanscomputermod.storage.core.StorageException;
import com.example.evanscomputermod.storage.device.DecoderBlockEntity;
import com.example.evanscomputermod.storage.device.DriveBlockEntity;
import com.example.evanscomputermod.storage.device.EncoderBlockEntity;
import com.example.evanscomputermod.storage.device.StorageModule;
import com.example.evanscomputermod.storage.item.StorageCellItem;
import com.example.evanscomputermod.storage.ledger.ItemTypes;
import com.example.evanscomputermod.storage.ledger.StorageLedger;
import com.example.evanscomputermod.testing.scenario.ScenarioRun;
import io.netty.buffer.Unpooled;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.component.DataComponents;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.item.component.ItemContainerContents;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.HopperBlock;
import net.minecraft.world.level.block.HorizontalDirectionalBlock;
import net.minecraft.world.level.block.entity.ChestBlockEntity;
import net.minecraft.world.level.block.entity.HopperBlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.function.BooleanSupplier;

/**
 * Item storage, namespace {@code ecm_storage} (1.21.1): the ledger through
 * the Encoder / Drive / Decoder, cell identity (duplicates, breaking a drive,
 * destruction), the recursion rule, tokens (single use, split / merge,
 * addresses, expiry and lost &amp; found), the Wired Bus Module and the bay
 * Storage Module, a Python program, item overlays, and the {@code storage}
 * app driven by a mouse click and by a Screen touch.
 */
@GameTestHolder(StorageTests.NS)
@PrefixGameTestTemplate(false)
public final class StorageTests {
    static final String NS = "ecm_storage";
    private static final String STRUCTURE = "gametest_storage";
    private static final long WALL_LIMIT_MS = 60_000;

    private static final BlockPos TERMINAL = new BlockPos(6, 1, 5);   // facing north: west = "left", east = "right"
    private static final BlockPos LEFT = TERMINAL.west();
    private static final BlockPos RIGHT = TERMINAL.east();
    private static final BlockPos CHEST = RIGHT.east();

    // ------------------------------------------------------------ ledger through the blocks

    /** A hopper feeds the encoder, which credits the drive's cell; a full cell pushes back. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".encoder")
    public static void encoder_fills_cell_and_pushes_back(GameTestHelper h) {
        Env e = new Env(h);
        BlockPos drive = new BlockPos(2, 1, 2), enc = new BlockPos(3, 1, 2), hopper = new BlockPos(3, 2, 2);
        e.run("encoder_fills_cell_and_pushes_back", List.of(
                () -> {
                    e.device(drive, StorageContent.DRIVE.get(), Direction.NORTH);
                    e.device(enc, StorageContent.ENCODER.get(), Direction.NORTH);
                    return true;
                },
                () -> e.drive(drive).isLive(),
                () -> {
                    e.drive(drive).cellHandler().setStackInSlot(0, e.cell(CellTier.K1));
                    return true;
                },
                () -> !e.encoder(enc).reach().isEmpty() && !e.drive(drive).cells().isEmpty(),
                () -> {
                    h.setBlock(hopper, Blocks.HOPPER.defaultBlockState().setValue(HopperBlock.FACING, Direction.DOWN));
                    ((HopperBlockEntity) h.getBlockEntity(hopper)).setItem(0, new ItemStack(Items.IRON_INGOT, 8));
                    return true;
                },
                () -> e.count(e.cellOf(drive, 0), new ItemStack(Items.IRON_INGOT)) == 8,
                () -> {
                    check(((HopperBlockEntity) h.getBlockEntity(hopper)).isEmpty(), "hopper kept items");
                    EncoderBlockEntity en = e.encoder(enc);
                    long cobble = 0;
                    for (int i = 0; i < 200; i++) {
                        ItemStack left = en.itemHandler().insertItem(0, new ItemStack(Items.COBBLESTONE, 64), false);
                        cobble += 64 - left.getCount();
                        if (!left.isEmpty()) break;
                    }
                    // 1k: 1024 bytes; 2 types x 8 bytes overhead; 8 items per byte.
                    check(cobble == (1024 - 16) * 8 - 8, "cobble stored " + cobble);
                    CellContents c = e.contents(e.cellOf(drive, 0));
                    check(c.usedBytes() == 1024, "used " + c.usedBytes());
                    ItemStack sim = en.itemHandler().insertItem(0, new ItemStack(Items.COBBLESTONE, 64), true);
                    check(sim.getCount() == 64, "full cell accepted a simulated insert: " + sim);
                    ItemStack other = en.itemHandler().insertItem(0, new ItemStack(Items.DIRT, 1), false);
                    check(other.getCount() == 1, "full cell took a new type");
                    return true;
                }));
    }

    /** extract() through the computer: into the chest in front, then (no chest) into the buffer until it's full. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".decoder")
    public static void decoder_extracts_into_chest_then_buffer(GameTestHelper h) {
        Env e = new Env(h);
        e.run("decoder_extracts_into_chest_then_buffer", List.of(
                e::computerWithDriveAndDecoder,
                () -> e.hub().names().contains("left") && e.hub().names().contains("right") && e.drive(LEFT).isLive(),
                () -> {
                    e.seed(e.cellOf(LEFT, 0), new ItemStack(Items.IRON_INGOT), 1000);
                    long n = (Long) e.num(e.call("left", "extract", e.key(Items.IRON_INGOT), 10));
                    check(n == 10, "extracted " + n);
                    ChestBlockEntity chest = (ChestBlockEntity) h.getBlockEntity(CHEST);
                    check(chest.getItem(0).is(Items.IRON_INGOT) && chest.getItem(0).getCount() == 10, "chest has " + chest.getItem(0));
                    check(e.count(e.cellOf(LEFT, 0), new ItemStack(Items.IRON_INGOT)) == 990, "cell not debited");
                    h.setBlock(CHEST, Blocks.AIR.defaultBlockState());
                    long m = (Long) e.num(e.call("right", "extract", e.key(Items.IRON_INGOT), 1000));
                    check(m == 9 * 64, "buffer took " + m);
                    check(e.count(e.cellOf(LEFT, 0), new ItemStack(Items.IRON_INGOT)) == 990 - 576, "cell not debited for the buffer");
                    long z = (Long) e.num(e.call("left", "extract", e.key(Items.IRON_INGOT), 5));
                    check(z == 0, "a full buffer still took " + z);
                    return true;
                }));
    }

    // ------------------------------------------------------------ cell identity

    /** A copy of a mounted cell (same id) is refused, then mounts once the original leaves. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".duplicate")
    public static void duplicate_cell_is_refused(GameTestHelper h) {
        Env e = new Env(h);
        BlockPos a = new BlockPos(2, 1, 2), b = new BlockPos(2, 1, 6);
        ItemStack[] cell = new ItemStack[1];
        e.run("duplicate_cell_is_refused", List.of(
                () -> {
                    e.device(a, StorageContent.DRIVE.get(), Direction.NORTH);
                    e.device(b, StorageContent.DRIVE.get(), Direction.NORTH);
                    return true;
                },
                () -> e.drive(a).isLive() && e.drive(b).isLive(),
                () -> {
                    cell[0] = e.cell(CellTier.K4);
                    e.drive(a).cellHandler().setStackInSlot(0, cell[0].copy());
                    return e.drive(a).status(0) == DriveBlockEntity.STATUS_OK;
                },
                () -> {
                    ItemStack copy = e.drive(a).cellHandler().getStackInSlot(0).copy();   // carries the id
                    e.drive(b).cellHandler().setStackInSlot(0, copy);
                    return true;
                },
                () -> {
                    check(e.drive(b).status(0) == DriveBlockEntity.STATUS_DUPLICATE, "copy status " + e.drive(b).status(0));
                    check(e.drive(b).cells().isEmpty(), "copy is mounted");
                    e.drive(a).cellHandler().setStackInSlot(0, ItemStack.EMPTY);
                    return true;
                },
                () -> e.drive(b).status(0) == DriveBlockEntity.STATUS_OK && e.drive(b).cells().size() == 1));
    }

    /** Breaking a drive drops its cells; the contents stay with the cell (picking it up doesn't void it). */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".break")
    public static void broken_drive_keeps_cell_contents(GameTestHelper h) {
        Env e = new Env(h);
        BlockPos a = new BlockPos(2, 1, 2), b = new BlockPos(8, 1, 8);
        UUID[] id = new UUID[1];
        ItemStack[] picked = new ItemStack[1];
        e.run("broken_drive_keeps_cell_contents", List.of(
                () -> {
                    e.device(a, StorageContent.DRIVE.get(), Direction.NORTH);
                    return true;
                },
                () -> e.drive(a).isLive(),
                () -> {
                    e.drive(a).cellHandler().setStackInSlot(3, e.cell(CellTier.K1));
                    return e.drive(a).cells().size() == 1;
                },
                () -> {
                    id[0] = e.drive(a).cells().get(0).id();
                    e.seed(id[0], new ItemStack(Items.DIAMOND), 50);
                    e.level().destroyBlock(h.absolutePos(a), true);
                    return true;
                },
                () -> {
                    for (ItemEntity it : e.itemsNear(a)) {
                        if (id[0].equals(StorageCellItem.cellId(it.getItem()))) {
                            picked[0] = it.getItem().copy();
                            it.getItem().setCount(0);     // what a pickup does before discarding
                            it.discard();
                            return true;
                        }
                    }
                    return false;
                },
                () -> {
                    check(e.ledger().get(id[0]) != null, "picking the cell up voided it");
                    e.device(b, StorageContent.DRIVE.get(), Direction.NORTH);
                    return true;
                },
                () -> e.drive(b).isLive(),
                () -> {
                    e.drive(b).cellHandler().setStackInSlot(0, picked[0]);
                    return e.drive(b).cells().size() == 1;
                },
                () -> e.count(id[0], new ItemStack(Items.DIAMOND)) == 50));
    }

    /** A cell item burnt in lava loses its contents. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".void")
    public static void destroyed_cell_is_voided(GameTestHelper h) {
        Env e = new Env(h);
        UUID id = UUID.randomUUID();
        e.run("destroyed_cell_is_voided", List.of(
                () -> {
                    e.ledger().ensureCell(id, CellTier.K1);
                    e.seed(id, new ItemStack(Items.GOLD_INGOT), 5);
                    ItemStack stack = e.cell(CellTier.K1);
                    stack.set(StorageContent.CELL_ID.get(), id);
                    Vec3 p = Vec3.atCenterOf(h.absolutePos(new BlockPos(4, 2, 4)));
                    ItemEntity it = new ItemEntity(e.level(), p.x, p.y, p.z, stack);
                    e.level().addFreshEntity(it);
                    it.hurt(e.level().damageSources().lava(), 100);
                    return true;
                },
                () -> e.ledger().get(id) == null));
    }

    /** A filled cell can't be encoded, nor a shulker holding one; an empty cell and an ordinary full shulker can. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".recursion")
    public static void filled_cells_cannot_be_nested(GameTestHelper h) {
        Env e = new Env(h);
        BlockPos drive = new BlockPos(2, 1, 2), enc = new BlockPos(3, 1, 2);
        e.run("filled_cells_cannot_be_nested", List.of(
                () -> {
                    e.device(drive, StorageContent.DRIVE.get(), Direction.NORTH);
                    e.device(enc, StorageContent.ENCODER.get(), Direction.NORTH);
                    return true;
                },
                () -> e.drive(drive).isLive(),
                () -> {
                    e.drive(drive).cellHandler().setStackInSlot(0, e.cell(CellTier.K4));
                    return !e.encoder(enc).reach().isEmpty() && !e.drive(drive).cells().isEmpty();
                },
                () -> {
                    UUID inner = UUID.randomUUID();
                    e.ledger().ensureCell(inner, CellTier.K1);
                    e.seed(inner, new ItemStack(Items.EMERALD), 3);
                    ItemStack filled = e.cell(CellTier.K1);
                    filled.set(StorageContent.CELL_ID.get(), inner);
                    EncoderBlockEntity en = e.encoder(enc);
                    check(en.encode(filled.copy(), false).getCount() == 1, "a filled cell was encoded");
                    ItemStack shulker = new ItemStack(Items.SHULKER_BOX);
                    shulker.set(DataComponents.CONTAINER, ItemContainerContents.fromItems(List.of(filled.copy())));
                    check(en.encode(shulker, false).getCount() == 1, "a shulker holding a filled cell was encoded");
                    check(en.encode(e.cell(CellTier.K1), false).isEmpty(), "an empty cell was refused");
                    ItemStack dirtBox = new ItemStack(Items.SHULKER_BOX);
                    dirtBox.set(DataComponents.CONTAINER, ItemContainerContents.fromItems(List.of(new ItemStack(Items.DIRT, 64))));
                    check(en.encode(dirtBox, false).isEmpty(), "a shulker of dirt was refused");
                    return true;
                }));
    }

    /** Sable assembles a Drive into a structure: its cell remounts in the moved block entity, contents intact. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".sable")
    public static void drive_moves_with_sable_structure(GameTestHelper h) {
        Env e = new Env(h);
        BlockPos drive = new BlockPos(8, 1, 8);
        UUID[] id = new UUID[1];
        String[] where = new String[1];
        e.run("drive_moves_with_sable_structure", List.of(
                () -> {
                    for (int x = 7; x <= 9; x++)
                        for (int z = 7; z <= 9; z++) h.setBlock(new BlockPos(x, 0, z), Blocks.STONE.defaultBlockState());
                    e.device(drive, StorageContent.DRIVE.get(), Direction.NORTH);
                    return true;
                },
                () -> e.drive(drive).isLive(),
                () -> {
                    e.drive(drive).cellHandler().setStackInSlot(2, e.cell(CellTier.K4));
                    return !e.drive(drive).cells().isEmpty();
                },
                () -> {
                    id[0] = e.cellOf(drive, 2);
                    e.seed(id[0], new ItemStack(Items.EMERALD), 25);
                    where[0] = e.ledger().mountOf(id[0]).describe();
                    List<BlockPos> blocks = new ArrayList<>();
                    BlockPos min = h.absolutePos(new BlockPos(7, 0, 7)), max = h.absolutePos(new BlockPos(9, 1, 9));
                    for (BlockPos p : BlockPos.betweenClosed(min, max))
                        if (!e.level().getBlockState(p).isAir()) blocks.add(p.immutable());
                    dev.ryanhcode.sable.api.SubLevelAssemblyHelper.assembleBlocks(e.level(), h.absolutePos(drive), blocks,
                            new dev.ryanhcode.sable.companion.math.BoundingBox3i(min, max));
                    check(!(e.level().getBlockState(h.absolutePos(drive)).getBlock() instanceof com.example.evanscomputermod.storage.device.DriveBlock),
                            "drive did not move");
                    return true;
                },
                () -> {
                    var m = e.ledger().mountOf(id[0]);
                    return m != null && m.isAlive() && !m.describe().equals(where[0]);
                },
                () -> {
                    check(e.count(id[0], new ItemStack(Items.EMERALD)) == 25, "contents changed in the move");
                    return true;
                }));
    }

    // ------------------------------------------------------------ tokens

    /** Tokens redeem once; split / merge conserve items; an addressed token only lands in its cell. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".tokens")
    public static void tokens_are_single_use(GameTestHelper h) {
        Env e = new Env(h);
        e.run("tokens_are_single_use", List.of(
                e::computerWithDriveAndDecoder,
                () -> e.hub().names().contains("left") && e.drive(LEFT).isLive(),
                () -> {
                    e.drive(LEFT).cellHandler().setStackInSlot(1, e.cell(CellTier.K1));
                    return e.drive(LEFT).cells().size() == 2;
                },
                () -> {
                    String a = e.cellOf(LEFT, 0).toString(), b = e.cellOf(LEFT, 1).toString();
                    e.seed(e.cellOf(LEFT, 0), new ItemStack(Items.IRON_INGOT), 40);
                    String k = e.key(Items.IRON_INGOT);
                    String t = (String) e.call("left", "withdraw_token", a, k, 10);
                    check(t.startsWith("ecmt1_") && t.length() == 38, "token " + t);
                    check(e.count(e.cellOf(LEFT, 0), new ItemStack(Items.IRON_INGOT)) == 30, "not debited");
                    check(e.num(((Map<?, ?>) e.call("left", "token_info", t)).get("count")) == 10, "info count");
                    e.call("left", "redeem", t, b);
                    check(e.count(e.cellOf(LEFT, 1), new ItemStack(Items.IRON_INGOT)) == 10, "not credited");
                    check(e.error("left", "redeem", t, b).contains("already spent"), "second redeem worked");

                    String t2 = (String) e.call("left", "withdraw_token", a, k, 9);
                    List<?> parts = (List<?>) e.call("left", "split", t2, List.of(4, 5));
                    check(parts.size() == 2, "split gave " + parts);
                    check(!e.error("left", "token_info", t2).isEmpty(), "split token still valid");
                    check(!e.error("left", "split", parts.get(0), List.of(1, 1)).isEmpty(), "split into the wrong total worked");
                    String t3 = (String) e.call("left", "merge", parts);
                    check(e.num(((Map<?, ?>) e.call("left", "token_info", t3)).get("count")) == 9, "merged count");

                    String t4 = (String) e.call("left", "withdraw_token", a, k, 3, b);
                    check(e.error("left", "redeem", t4, a).contains("addressed"), "addressed token redeemed elsewhere");
                    e.call("left", "redeem", t4);
                    check(e.count(e.cellOf(LEFT, 1), new ItemStack(Items.IRON_INGOT)) == 13, "addressed token not credited to its cell");
                    e.call("left", "redeem", t3, a);
                    check(e.count(e.cellOf(LEFT, 0), new ItemStack(Items.IRON_INGOT)) + e.count(e.cellOf(LEFT, 1),
                            new ItemStack(Items.IRON_INGOT)) == 40, "items not conserved");
                    return true;
                }));
    }

    /** An expired token goes back to its cell; if the cell is full, to lost &amp; found, which claim_lost empties. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".expiry")
    public static void expired_tokens_refund_or_go_to_lost(GameTestHelper h) {
        Env e = new Env(h);
        String[] t = new String[2];
        e.run("expired_tokens_refund_or_go_to_lost", List.of(
                e::computerWithDriveAndDecoder,
                () -> e.hub().names().contains("left") && e.drive(LEFT).isLive() && !e.drive(LEFT).cells().isEmpty(),
                () -> {
                    e.seed(e.cellOf(LEFT, 0), new ItemStack(Items.IRON_INGOT), 20);
                    t[0] = (String) e.call("left", "withdraw_token", e.cellOf(LEFT, 0).toString(), e.key(Items.IRON_INGOT), 5, null, 1);
                    return true;
                },
                () -> e.ledger().tokens().get(t[0]) == null,
                () -> {
                    check(e.count(e.cellOf(LEFT, 0), new ItemStack(Items.IRON_INGOT)) == 20, "not refunded");
                    t[1] = (String) e.call("left", "withdraw_token", e.cellOf(LEFT, 0).toString(), e.key(Items.IRON_INGOT), 5, null, 1);
                    // Fill the cell so the refund has nowhere to go.
                    int cobble = e.intern(new ItemStack(Items.COBBLESTONE));
                    CellContents c = e.contents(e.cellOf(LEFT, 0));
                    e.add(c, cobble, c.maxInsert(cobble));
                    return true;
                },
                () -> e.ledger().tokens().get(t[1]) == null,
                () -> {
                    List<?> lost = (List<?>) e.call("left", "lost");
                    check(lost.size() == 1 && e.num(((Map<?, ?>) lost.get(0)).get("count")) == 5, "lost " + lost);
                    CellContents c = e.contents(e.cellOf(LEFT, 0));
                    try {
                        c.remove(e.intern(new ItemStack(Items.COBBLESTONE)), 100);
                    } catch (StorageException ex) {
                        throw new AssertionError(ex.getMessage());
                    }
                    check(e.num(e.call("left", "claim_lost")) == 5, "claim_lost");
                    check(e.count(e.cellOf(LEFT, 0), new ItemStack(Items.IRON_INGOT)) == 20, "lost items not back");
                    check(((List<?>) e.call("left", "lost")).isEmpty(), "lost not emptied");
                    return true;
                }));
    }

    // ------------------------------------------------------------ reaching storage

    /** A Drive wired to the Wired Bus Module joins the computer's storage net. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".wired")
    public static void wired_drive_is_reached_through_the_bus(GameTestHelper h) {
        Env e = new Env(h);
        BlockPos drive = new BlockPos(3, 1, 5);   // two blocks west of the terminal, back (connector) facing it
        e.run("wired_drive_is_reached_through_the_bus", List.of(
                () -> {
                    // Sensor Wire routes along surfaces: give it a floor.
                    for (int x = 0; x < 16; x++)
                        for (int z = 0; z < 12; z++) h.setBlock(new BlockPos(x, 0, z), Blocks.STONE.defaultBlockState());
                    e.placeTerminal();
                    e.device(drive, StorageContent.DRIVE.get(), Direction.WEST);
                    return true;
                },
                () -> e.drive(drive).isLive(),
                () -> {
                    e.drive(drive).cellHandler().setStackInSlot(0, e.cell(CellTier.K1));
                    e.hold(new ItemStack(ModItems.MODULE_EXPANSION_CARD.get()));
                    e.useOn(Direction.WEST, 0.5);
                    e.hold(new ItemStack(SensorContent.WIRED_SENSOR_MODULE.get()));
                    e.useOn(Direction.WEST, 0.75);
                    return e.hub().names().contains("left_bay_1");
                },
                () -> {
                    e.hold(new ItemStack(SensorContent.SENSOR_WIRE.get(), 64));
                    e.clickConnector(h.absolutePos(TERMINAL), 0, Direction.WEST);
                    e.clickConnector(h.absolutePos(drive), 0, Direction.EAST);
                    return true;
                },
                () -> ((Map<?, ?>) e.call("left_bay_1", "storage_devices")).containsKey("drive_1"),
                () -> {
                    List<?> cells = (List<?>) e.call("left_bay_1", "cells");
                    check(cells.size() == 1, "cells through the bus: " + cells);
                    e.seed(e.cellOf(drive, 0), new ItemStack(Items.REDSTONE), 7);
                    check(e.num(e.call("left_bay_1", "total", e.key(Items.REDSTONE))) == 7, "total through the bus");
                    return true;
                }));
    }

    /** A Storage Module in a bay takes a cell (right-click the slot) and gives it back first on sneak-click. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".module")
    public static void bay_storage_module_holds_a_cell(GameTestHelper h) {
        Env e = new Env(h);
        e.run("bay_storage_module_holds_a_cell", List.of(
                () -> {
                    e.placeTerminal();
                    return true;
                },
                () -> {
                    e.hold(new ItemStack(ModItems.MODULE_EXPANSION_CARD.get()));
                    e.useOn(Direction.WEST, 0.5);
                    e.hold(new ItemStack(StorageContent.STORAGE_MODULE.get()));
                    e.useOn(Direction.WEST, 0.75);
                    return e.hub().names().contains("left_bay_1");
                },
                () -> {
                    ItemStack cell = e.hold(e.cell(CellTier.K16));
                    e.useOn(Direction.WEST, 0.75);
                    check(cell.isEmpty(), "cell not taken");
                    return e.terminal().getModuleBays().getModule(0) instanceof StorageModule m && !m.cells().isEmpty();
                },
                () -> {
                    List<?> cells = (List<?>) e.call("left_bay_1", "cells");
                    check(cells.size() == 1 && "module".equals(((Map<?, ?>) cells.get(0)).get("device")), "cells " + cells);
                    e.hold(ItemStack.EMPTY);
                    e.player.setShiftKeyDown(true);
                    e.useOn(Direction.WEST, 0.75);
                    check(e.count(StorageContent.CELLS.get(CellTier.K16).get()) == 1, "cell not given back");
                    check(!e.terminal().getModuleBays().getStack(0).isEmpty(), "module came out before its cell");
                    e.player.getInventory().clearContent();
                    e.hold(ItemStack.EMPTY);
                    e.useOn(Direction.WEST, 0.75);
                    check(e.terminal().getModuleBays().getStack(0).isEmpty(), "module not removed");
                    e.player.setShiftKeyDown(false);
                    return true;
                }));
    }

    // ------------------------------------------------------------ programs

    /** A Python program lists, extracts and spends a token through the storage module. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".python")
    public static void python_program_uses_storage(GameTestHelper h) {
        Env e = new Env(h);
        String script = String.join("\n",
                "import storage, peripheral",
                "n = storage.net()",
                "items = n.items()",
                "print('ITEMS', [(i['id'], i['count']) for i in items])",
                "k = n.find('minecraft:iron_ingot')[0]",
                "print('OUT', n.extract(k, 3))",
                "c = n.cells()[0]['id']",
                "t = n.withdraw_token(c, k, 2)",
                "print('R1', n.redeem(t)['count'])",
                "try:",
                "    n.redeem(t)",
                "except peripheral.PeripheralError as err:",
                "    print('R2', err)",
                "print('LEFT', n.total(k))",
                "print('DONE')",
                "");
        e.run("python_program_uses_storage", List.of(
                e::computerWithDriveAndDecoder,
                () -> e.drive(LEFT).isLive() && !e.drive(LEFT).cells().isEmpty(),
                () -> {
                    e.seed(e.cellOf(LEFT, 0), new ItemStack(Items.IRON_INGOT), 30);
                    e.terminal().initializeWasm();
                    return true;
                },
                () -> e.screen().contains("Welcome to Terminal OS"),
                () -> e.runScript("storage_test.py", script),
                () -> e.screen().contains("DONE"),
                () -> {
                    String s = e.screen();
                    check(s.contains("ITEMS [('minecraft:iron_ingot', 30)]"), "items:\n" + s);
                    check(s.contains("OUT 3"), "extract:\n" + s);
                    check(s.contains("R1 2"), "redeem:\n" + s);
                    check(s.contains("R2 unknown or already spent token"), "double redeem:\n" + s);
                    check(s.contains("LEFT 27"), "total:\n" + s);
                    ChestBlockEntity chest = (ChestBlockEntity) h.getBlockEntity(CHEST);
                    check(chest.getItem(0).getCount() == 3, "chest " + chest.getItem(0));
                    return true;
                }));
    }

    /** Overlay lists resolve item ids and storage keys, skip unknown items, and survive the packet codec. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".overlay")
    public static void item_overlay_resolves_and_encodes(GameTestHelper h) {
        Env e = new Env(h);
        e.run("item_overlay_resolves_and_encodes", List.of(() -> {
            ItemStack sword = new ItemStack(Items.DIAMOND_SWORD);
            sword.setDamageValue(7);
            int key = e.intern(sword);
            ItemOverlays o = new ItemOverlays();
            byte[] list = overlayList(new Object[][] {
                    {4, 6, 16, "minecraft:diamond", "64", 0x80},
                    {24, 6, 32, ItemTypes.keyOf(key), "1", 0},
                    {44, 6, 16, "nosuchmod:thing", "", 0}});
            check(o.set(ItemOverlays.TARGET_TERMINAL, list, 5) == 2, "kept " + o.entries(ItemOverlays.TARGET_TERMINAL));
            check(o.set(ItemOverlays.TARGET_TERMINAL, new byte[] {9, 9, 9}, 5) == -1, "a malformed list was accepted");
            check(o.set(ItemOverlays.TARGET_TERMINAL, list, 5) == 2, "second set");
            var entries = o.entries(ItemOverlays.TARGET_TERMINAL);
            check(entries.get(0).flags() == 0x80 && "64".equals(entries.get(0).label()), "entry " + entries.get(0));
            ItemOverlayPacket packet = new ItemOverlayPacket(h.absolutePos(TERMINAL), (byte) 0, 3, entries,
                    Map.of(entries.get(0).proto(), new ItemStack(Items.DIAMOND), entries.get(1).proto(), sword));
            RegistryFriendlyByteBuf buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), e.level().registryAccess());
            ItemOverlayPacket.STREAM_CODEC.encode(buf, packet);
            ItemOverlayPacket back = ItemOverlayPacket.STREAM_CODEC.decode(buf);
            check(back.entries().equals(entries), "entries changed in transit");
            check(ItemStack.isSameItemSameComponents(back.protos().get(entries.get(1).proto()), sword), "stack changed in transit");
            check(o.clearOwnedBy(5) && o.entries(ItemOverlays.TARGET_TERMINAL).isEmpty(), "exit didn't clear");
            return true;
        }));
    }

    /** The storage app on the terminal: a left click on the first item sends one out of the decoder. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".app")
    public static void storage_app_click_requests_an_item(GameTestHelper h) {
        Env e = new Env(h);
        e.run("storage_app_click_requests_an_item", List.of(
                e::computerWithDriveAndDecoder,
                () -> e.drive(LEFT).isLive() && !e.drive(LEFT).cells().isEmpty(),
                () -> {
                    e.seed(e.cellOf(LEFT, 0), new ItemStack(Items.IRON_INGOT), 30);
                    e.terminal().initializeWasm();
                    return true;
                },
                () -> e.screen().contains("Welcome to Terminal OS"),
                () -> {
                    e.terminal().onStringInput("storage\n");
                    return true;
                },
                () -> !e.terminal().getItemOverlays().entries(ItemOverlays.TARGET_TERMINAL).isEmpty(),
                () -> {
                    var entry = e.terminal().getItemOverlays().entries(ItemOverlays.TARGET_TERMINAL).get(0);
                    short x = (short) (entry.x() + entry.size() / 2), y = (short) (entry.y() + entry.size() / 2);
                    BlockPos pos = h.absolutePos(TERMINAL);
                    e.terminal().onMouseEvent(new MouseInputPacket(pos, MouseInputPacket.KIND_MOVE, x, y, (byte) 0, (byte) 0, (byte) 0, java.util.Optional.empty()));
                    e.terminal().onMouseEvent(new MouseInputPacket(pos, MouseInputPacket.KIND_DOWN, x, y, (byte) 1, (byte) 0, (byte) 0, java.util.Optional.empty()));
                    e.terminal().onMouseEvent(new MouseInputPacket(pos, MouseInputPacket.KIND_UP, x, y, (byte) 0, (byte) 0, (byte) 0, java.util.Optional.empty()));
                    return true;
                },
                () -> {
                    ChestBlockEntity chest = (ChestBlockEntity) h.getBlockEntity(CHEST);
                    return chest.getItem(0).is(Items.IRON_INGOT) && chest.getItem(0).getCount() == 1;
                },
                () -> e.count(e.cellOf(LEFT, 0), new ItemStack(Items.IRON_INGOT)) == 29));
    }

    /**
     * The storage app on a 3x2 Screen cluster: the whole framebuffer gets drawn, and a right-click (touch) on
     * an item opens the request dialog (the items dim).
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".touch")
    public static void storage_app_screen_touch(GameTestHelper h) {
        Env e = new Env(h);
        long[] gen = new long[1];
        e.run("storage_app_screen_touch", List.of(
                e::computerWithDriveAndDecoder,
                () -> {
                    // A 3 wide, 2 high cluster facing north, standing on the terminal (its front is its own display).
                    for (int dx = -1; dx <= 1; dx++)
                        for (int dy = 1; dy <= 2; dy++)
                            h.setBlock(TERMINAL.offset(dx, dy, 0), ModBlocks.SCREEN_BLOCK.get().defaultBlockState()
                                    .setValue(ScreenBlock.FACING, Direction.NORTH));
                    return true;
                },
                () -> e.drive(LEFT).isLive() && !e.drive(LEFT).cells().isEmpty() && e.terminal().hasScreenCluster()
                        && e.terminal().getScreenClusterInfo().cols() == 3,
                () -> {
                    e.seed(e.cellOf(LEFT, 0), new ItemStack(Items.IRON_INGOT), 30);
                    e.seed(e.cellOf(LEFT, 0), new ItemStack(Items.GOLD_INGOT), 12);
                    e.terminal().initializeWasm();
                    return true;
                },
                () -> e.screen().contains("Welcome to Terminal OS"),
                () -> {
                    e.terminal().onStringInput("storage --screen\n");
                    return true;
                },
                () -> e.terminal().getItemOverlays().entries(ItemOverlays.TARGET_SCREEN).size() == 2,
                () -> {
                    var d = e.terminal().getScreenDisplay();
                    var info = e.terminal().getScreenClusterInfo();
                    check(d.getGfxWidth() == info.gfxWidth() && d.getGfxHeight() == info.gfxHeight(),
                            "screen display " + d.getGfxWidth() + "x" + d.getGfxHeight() + ", cluster " + info.gfxWidth() + "x" + info.gfxHeight());
                    check(d.getPixelFormat() == com.example.evanscomputermod.computer.TerminalDisplay.PIXEL_FORMAT_RGBA8888,
                            "screen not RGBA: " + d.getPixelFormat());
                    byte[] px = d.getPixelData();
                    int unset = 0, w = d.getGfxWidth(), hh = d.getGfxHeight();
                    int minX = w, minY = hh, maxX = -1, maxY = -1;
                    for (int i = 0; i < w * hh; i++) {
                        if (px[i * 4 + 3] == 0) {
                            unset++;
                            int x = i % w, y = i / w;
                            minX = Math.min(minX, x); minY = Math.min(minY, y); maxX = Math.max(maxX, x); maxY = Math.max(maxY, y);
                        }
                    }
                    check(unset == 0, unset + " of " + (w * hh) + " screen pixels never drawn, in x " + minX + ".." + maxX
                            + " y " + minY + ".." + maxY + " (" + w + "x" + hh + ")");
                    return true;
                },
                () -> {
                    ItemOverlays o = e.terminal().getItemOverlays();
                    gen[0] = o.generation(ItemOverlays.TARGET_SCREEN);
                    var entry = o.entries(ItemOverlays.TARGET_SCREEN).get(0);
                    var d = e.terminal().getScreenDisplay();
                    var info = e.terminal().getScreenClusterInfo();
                    BlockPos a = info.anchor();
                    double u = (entry.x() + entry.size() / 2.0) / d.getGfxWidth() * info.cols();
                    double v = (entry.y() + entry.size() / 2.0) / d.getGfxHeight() * info.rows();
                    // A north-facing picture runs from the anchor's top-right corner (x+1) westwards, top to bottom.
                    Vec3 at = new Vec3(a.getX() + 1 - u, a.getY() + 1 - v, a.getZ());
                    BlockPos hitBlock = BlockPos.containing(at.x, at.y, a.getZ());
                    BlockState st = e.level().getBlockState(hitBlock);
                    var r = ScreenTouch.use(st, e.level(), hitBlock, e.player, new BlockHitResult(at, Direction.NORTH, hitBlock, false));
                    check(r.consumesAction(), "touch not taken: " + r);
                    return true;
                },
                () -> {
                    ItemOverlays o = e.terminal().getItemOverlays();
                    if (o.generation(ItemOverlays.TARGET_SCREEN) == gen[0]) return false;
                    var entries = o.entries(ItemOverlays.TARGET_SCREEN);
                    // The dialog covers the grid: its items are dropped or dimmed.
                    return entries.stream().allMatch(en -> (en.flags() & ItemOverlays.FLAG_DIMMED) != 0);
                }));
    }

    // ------------------------------------------------------------ plumbing

    private static byte[] overlayList(Object[][] rows) {
        ByteBuffer b = ByteBuffer.allocate(4096).order(ByteOrder.LITTLE_ENDIAN);
        b.putShort((short) rows.length);
        for (Object[] r : rows) {
            b.putShort((short) (int) r[0]).putShort((short) (int) r[1]).putShort((short) (int) r[2]);
            b.putShort((short) 0).putShort((short) 0).putShort((short) 320).putShort((short) 200);
            b.put((byte) (int) r[5]);
            byte[] ref = ((String) r[3]).getBytes(StandardCharsets.UTF_8);
            b.put((byte) ref.length).put(ref);
            byte[] label = ((String) r[4]).getBytes(StandardCharsets.UTF_8);
            b.put((byte) label.length).put(label);
        }
        byte[] out = new byte[b.position()];
        b.flip().get(out);
        return out;
    }

    private static void check(boolean ok, String message) {
        if (!ok) throw new AssertionError(message);
    }

    /** One test's world and a step list: each step runs every tick until it returns true. */
    private static final class Env {
        final GameTestHelper h;
        final Player player;
        private int step;
        private long started;
        private String failure;

        Env(GameTestHelper h) {
            this.h = h;
            this.player = h.makeMockPlayer(GameType.SURVIVAL);
        }

        void run(String name, List<BooleanSupplier> steps) {
            List<BooleanSupplier> all = new ArrayList<>(steps);
            TestDriver.drive(h, NS, name, () -> {
                if (started == 0) started = System.currentTimeMillis();
                if (System.currentTimeMillis() - started > WALL_LIMIT_MS) {
                    failure = "timed out at step " + (step + 1) + "/" + all.size() + "\n" + screen();
                    return false;
                }
                try {
                    while (step < all.size() && all.get(step).getAsBoolean()) step++;
                } catch (AssertionError | RuntimeException ex) {
                    failure = "step " + (step + 1) + ": " + ex + "\n" + screen();
                    return false;
                }
                return step >= all.size();
            }, () -> failure);
        }

        ServerLevel level() {
            return h.getLevel();
        }

        StorageLedger ledger() {
            return StorageLedger.get(level().getServer());
        }

        void device(BlockPos rel, Block block, Direction facing) {
            h.setBlock(rel, block.defaultBlockState().setValue(HorizontalDirectionalBlock.FACING, facing));
        }

        DriveBlockEntity drive(BlockPos rel) {
            return (DriveBlockEntity) h.getBlockEntity(rel);
        }

        EncoderBlockEntity encoder(BlockPos rel) {
            return (EncoderBlockEntity) h.getBlockEntity(rel);
        }

        ItemStack cell(CellTier tier) {
            return new ItemStack(StorageContent.CELLS.get(tier).get());
        }

        UUID cellOf(BlockPos drive, int slot) {
            UUID id = StorageCellItem.cellId(drive(drive).cellHandler().getStackInSlot(slot));
            check(id != null, "no cell id in slot " + slot);
            return id;
        }

        CellContents contents(UUID cell) {
            CellContents c = ledger().get(cell);
            check(c != null, "no ledger record for " + cell);
            return c;
        }

        int intern(ItemStack stack) {
            try {
                return ledger().intern(stack);
            } catch (StorageException ex) {
                throw new AssertionError(ex.getMessage());
            }
        }

        void add(CellContents c, int key, long n) {
            try {
                c.add(key, n);
            } catch (StorageException ex) {
                throw new AssertionError(ex.getMessage());
            }
        }

        void seed(UUID cell, ItemStack type, long n) {
            add(contents(cell), intern(type), n);
            ledger().changed(cell);
        }

        long count(UUID cell, ItemStack type) {
            CellContents c = ledger().get(cell);
            int k = ledger().types().find(type);
            return c == null || k == 0 ? 0 : c.count(k);
        }

        String key(net.minecraft.world.item.Item item) {
            return ItemTypes.keyOf(intern(new ItemStack(item)));
        }

        long num(Object o) {
            check(o instanceof Number, "not a number: " + o);
            return ((Number) o).longValue();
        }

        List<ItemEntity> itemsNear(BlockPos rel) {
            BlockPos p = h.absolutePos(rel);
            return level().getEntitiesOfClass(ItemEntity.class, new AABB(p).inflate(2), it -> !it.isRemoved());
        }

        // ---- computer

        void placeTerminal() {
            h.setBlock(TERMINAL, ModBlocks.TERMINAL_BLOCK.get().defaultBlockState().setValue(TerminalBlock.FACING, Direction.NORTH));
        }

        /** Terminal; Drive (with a 1k cell) on its left; Decoder on its right facing a chest. */
        boolean computerWithDriveAndDecoder() {
            placeTerminal();
            device(LEFT, StorageContent.DRIVE.get(), Direction.NORTH);
            device(RIGHT, StorageContent.DECODER.get(), Direction.EAST);
            h.setBlock(CHEST, Blocks.CHEST.defaultBlockState());
            drive(LEFT).cellHandler().setStackInSlot(0, cell(CellTier.K1));
            return true;
        }

        TerminalBlockEntity terminal() {
            return (TerminalBlockEntity) h.getBlockEntity(TERMINAL);
        }

        com.example.evanscomputermod.computer.peripheral.PeripheralHub hub() {
            return terminal().getPeripheralHub();
        }

        String screen() {
            return h.getBlockEntity(TERMINAL) instanceof TerminalBlockEntity t ? ScenarioRun.screen(t.getDisplay()) : "";
        }

        boolean runScript(String file, String script) {
            Path dir = Path.of("computer-data", terminal().getComputerId().toString());
            try {
                Files.createDirectories(dir);
                Files.writeString(dir.resolve(file), script);
            } catch (java.io.IOException ex) {
                throw new AssertionError("can't write the script: " + ex);
            }
            terminal().onStringInput("python " + file + "\n");
            return true;
        }

        byte[] rawCall(String name, String method, Object... args) {
            java.util.List<Object> list = new ArrayList<>(java.util.Arrays.asList(args));
            return hub().call(name, method, PeripheralValues.encode(list), level().getServer());
        }

        /** Call a peripheral method through the hub, as a program would; errors throw. */
        Object call(String name, String method, Object... args) {
            byte[] frame = rawCall(name, method, args);
            try {
                Object v = PeripheralValues.decode(java.util.Arrays.copyOfRange(frame, 1, frame.length));
                if (frame[0] != PeripheralValues.STATUS_OK) throw new AssertionError(method + " failed: " + v);
                return v;
            } catch (PeripheralValues.DecodeException ex) {
                throw new AssertionError("bad frame from " + method + ": " + ex.getMessage());
            }
        }

        /** The error message of a call that should fail ("" if it succeeded). */
        String error(String name, String method, Object... args) {
            byte[] frame = rawCall(name, method, args);
            if (frame[0] == PeripheralValues.STATUS_OK) return "";
            try {
                return String.valueOf(PeripheralValues.decode(java.util.Arrays.copyOfRange(frame, 1, frame.length)));
            } catch (PeripheralValues.DecodeException ex) {
                return "undecodable error";
            }
        }

        // ---- player actions

        ItemStack hold(ItemStack stack) {
            player.setItemInHand(InteractionHand.MAIN_HAND, stack);
            return stack;
        }

        int count(net.minecraft.world.item.Item item) {
            int n = 0;
            for (ItemStack s : player.getInventory().items) if (s.is(item)) n += s.getCount();
            return n;
        }

        private BlockHitResult hit(Direction face, double y) {
            BlockPos abs = h.absolutePos(TERMINAL);
            Vec3 c = Vec3.atCenterOf(abs);
            Vec3 at = new Vec3(c.x + face.getStepX() * 0.5, abs.getY() + y, c.z + face.getStepZ() * 0.5);
            return new BlockHitResult(at, face, abs, false);
        }

        /** Right-click the terminal with the held item, the way the server does it. */
        void useOn(Direction face, double y) {
            BlockState st = h.getBlockState(TERMINAL);
            ItemStack held = player.getMainHandItem();
            var result = st.useItemOn(held, level(), player, InteractionHand.MAIN_HAND, hit(face, y));
            if (result == net.minecraft.world.ItemInteractionResult.PASS_TO_DEFAULT_BLOCK_INTERACTION) {
                st.useWithoutItem(level(), player, hit(face, y));
            }
        }

        /** Click a wire host's connector {@code index} with the held Sensor Wire. */
        void clickConnector(BlockPos absPos, int index, Direction face) {
            var host = IWireHost.getAt(level(), absPos);
            check(host != null, "no wire host at " + absPos);
            var terminal = host.terminal(level().getBlockState(absPos), index);
            check(terminal != null, "no connector " + index + " at " + absPos);
            Vec3 o = terminal.getOrigin();
            Vec3 at = new Vec3(absPos.getX() + o.x - face.getStepX() * 0.02, absPos.getY() + o.y - face.getStepY() * 0.02,
                    absPos.getZ() + o.z - face.getStepZ() * 0.02);
            var r = SensorWireItem.useOnBlock(player, player.getMainHandItem(), new BlockHitResult(at, face, absPos, false));
            check(r.consumesAction(), "click on connector " + index + " at " + absPos + " gave " + r);
        }
    }

    private StorageTests() {}
}
//?}
