package com.example.evanscomputermod.radio.compat.aero;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.radio.compat.sable.SableShips;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.neoforged.fml.ModList;

import java.lang.reflect.Method;
import java.util.List;

/**
 * Create Aeronautics (and Create Simulated, its base) for radio (spec: Sable and
 * Create Aeronautics support). Nothing here is a compile-time dependency: blocks
 * are looked up by id and Simulated's assembly helper is called reflectively, so
 * the mod builds and loads without Aeronautics and the radio data for its blocks
 * ({@code rf_attenuation} entries, conductor and ground tags) is shipped as
 * optional data that only applies when the mods are loaded.
 *
 * <p>Airships are Sable sub-levels: with Simulated present, a scenario ship is
 * assembled and set down by Simulated's own helper (the path its Physics
 * Assembler uses); otherwise by Sable's.
 */
public final class AeroCompat {

    public static final String AERONAUTICS = "aeronautics", SIMULATED = "simulated";

    private AeroCompat() {}

    public static boolean aeronautics() {
        return loaded(AERONAUTICS);
    }

    public static boolean simulated() {
        return loaded(SIMULATED);
    }

    private static boolean loaded(String mod) {
        try {
            return ModList.get() != null && ModList.get().isLoaded(mod);
        } catch (Throwable t) {
            return false;
        }
    }

    /** A block by id, or null when its mod is absent. */
    public static Block block(String id) {
        ResourceLocation rl = ResourceLocation.parse(id);
        return BuiltInRegistries.BLOCK.containsKey(rl) ? BuiltInRegistries.BLOCK.get(rl) : null;
    }

    /**
     * Balloon fabric: an Aeronautics envelope of the given colour when Aeronautics
     * is loaded, else wool of the same colour (what a ship without the mod would
     * use, and RF-wise the same material).
     */
    public static BlockState envelope(String color) {
        Block b = aeronautics() ? block("aeronautics:" + color + "_envelope") : null;
        if (b != null) return b.defaultBlockState();
        Block wool = block("minecraft:" + color + "_wool");
        return (wool == null ? Blocks.WHITE_WOOL : wool).defaultBlockState();
    }

    /** True if the block is an Aeronautics envelope. */
    public static boolean isEnvelope(BlockState s) {
        ResourceLocation id = BuiltInRegistries.BLOCK.getKey(s.getBlock());
        return id.getNamespace().equals(AERONAUTICS) && id.getPath().contains("envelope");
    }

    /** Metal frame for a ship's keel: Create's metal girder when Create is loaded, else iron bars. */
    public static BlockState metalFrame() {
        Block g = block("create:metal_girder");
        return (g == null ? Blocks.IRON_BARS : g).defaultBlockState();
    }

    // ------------------------------------------------------------ assembly

    /** How the last {@link #assemble} built its ship (for logs and receipts). */
    public static volatile String lastAssembler = "none";

    /**
     * Assemble the connected structure containing {@code anchor} into an airship.
     * With Simulated: {@code SimAssemblyHelper.assembleFromSingleBlock} (the Physics
     * Assembler's path, which gathers every connected block). Without: Sable's
     * {@code SubLevelAssemblyHelper} on {@code blocks}. Null if Sable is absent.
     */
    public static SableShips.Ship assemble(ServerLevel level, BlockPos anchor, List<BlockPos> blocks, BlockPos min, BlockPos max) {
        java.util.Map<BlockPos, BlockState> before = new java.util.HashMap<>();
        for (BlockPos p : blocks) before.put(p.immutable(), level.getBlockState(p));
        if (!before.containsKey(anchor)) before.put(anchor.immutable(), level.getBlockState(anchor));
        SableShips.Ship ship = assembleRaw(level, anchor, blocks, min, max);
        SableShips.Ship cal = SableShips.calibrate(ship, before);
        if (cal != ship) {
            EvansComputerMod.LOGGER.info("[aero] {}: block offset corrected from {} to {}", lastAssembler, ship.plotPos(BlockPos.ZERO), cal.plotPos(BlockPos.ZERO));
            lastAssembler += ", offset corrected";
        }
        return cal;
    }

    private static SableShips.Ship assembleRaw(ServerLevel level, BlockPos anchor, List<BlockPos> blocks, BlockPos min, BlockPos max) {
        if (simulated()) {
            try {
                // Like a player does before pulling the Physics Assembler's lever: super glue over the
                // whole ship, so Simulated's connected-block search takes every block, not just attached ones.
                Glue.cover(level, min, max);
                Class<?> helper = Class.forName("dev.simulated_team.simulated.util.SimAssemblyHelper");
                Method m = helper.getMethod("assembleFromSingleBlock", net.minecraft.world.level.Level.class, BlockPos.class,
                        BlockPos.class, boolean.class, boolean.class);
                Object result = m.invoke(null, level, anchor, anchor, true, true);
                if (result != null) {
                    Object sub = result.getClass().getMethod("subLevel").invoke(result);
                    // Simulated centres the plot on the first block it gathered and reports the block offset.
                    BlockPos offset = (BlockPos) result.getClass().getMethod("offset").invoke(result);
                    SableShips.Ship ship = SableShips.wrap(level, sub, anchor, offset);
                    lastAssembler = "Create Simulated SimAssemblyHelper (offset " + offset + ")";
                    return ship;
                }
                EvansComputerMod.LOGGER.warn("[aero] Simulated assembly returned nothing at {}; using Sable", anchor);
            } catch (ReflectiveOperationException | RuntimeException e) {
                EvansComputerMod.LOGGER.warn("[aero] Simulated assembly unavailable ({}); using Sable", e.toString());
            }
        }
        SableShips.Ship ship = SableShips.assemble(level, anchor, blocks, min, max);
        lastAssembler = ship == null ? "none (Sable not loaded)" : "Sable SubLevelAssemblyHelper";
        return ship;
    }

    /** Create classes, only touched when Simulated (which needs Create) is loaded. */
    private static final class Glue {
        static void cover(ServerLevel level, BlockPos min, BlockPos max) {
            level.addFreshEntity(new com.simibubi.create.content.contraptions.glue.SuperGlueEntity(level,
                    new net.minecraft.world.phys.AABB(min.getX(), min.getY(), min.getZ(), max.getX() + 1, max.getY() + 1, max.getZ() + 1)));
        }
    }

    /** How the last {@link #disassemble} set its ship down. */
    public static volatile String lastDisassembler = "none";

    /**
     * Set the ship down so its anchor block lands on {@code goal}. With Simulated:
     * {@code SimAssemblyHelper.disassembleSubLevel} (the Physics Assembler's path);
     * without: Sable's block move.
     */
    public static void disassemble(SableShips.Ship ship, BlockPos goal) {
        SableShips.release(ship);
        if (simulated()) {
            try {
                Class<?> helper = Class.forName("dev.simulated_team.simulated.util.SimAssemblyHelper");
                Class<?> subLevel = Class.forName("dev.ryanhcode.sable.sublevel.SubLevel");
                Method m = helper.getMethod("disassembleSubLevel", net.minecraft.world.level.Level.class, subLevel,
                        BlockPos.class, BlockPos.class, net.minecraft.world.level.block.Rotation.class, boolean.class);
                m.invoke(null, ship.level(), SableShips.subLevel(ship), ship.plotPos(SableShips.anchor(ship)), goal,
                        net.minecraft.world.level.block.Rotation.NONE, false);
                lastDisassembler = "Create Simulated SimAssemblyHelper.disassembleSubLevel";
                return;
            } catch (ReflectiveOperationException | RuntimeException e) {
                EvansComputerMod.LOGGER.warn("[aero] Simulated disassembly unavailable ({}); using Sable", e.toString());
            }
        }
        int n = SableShips.disassemble(ship, goal);
        lastDisassembler = "Sable block move (" + n + " blocks)";
    }
}
//?}
