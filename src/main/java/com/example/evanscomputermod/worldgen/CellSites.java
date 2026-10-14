package com.example.evanscomputermod.worldgen;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.*;
import com.example.evanscomputermod.computer.CellPlan;
import com.example.evanscomputermod.computer.FiberChords;
import com.example.evanscomputermod.computer.WorldNetwork;
import com.example.evanscomputermod.radio.antenna.RadioAntennaContent;
import com.example.evanscomputermod.radio.cellular.CellAntennaBlock;
import com.example.evanscomputermod.radio.cellular.CellularContent;
import com.example.evanscomputermod.radio.conductor.ConductorBlock;
import com.example.evanscomputermod.radio.sdr.RadioSdrContent;
import com.example.evanscomputermod.radio.sdr.SdrBlock;

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.tags.BlockTags;
import net.minecraft.world.level.WorldGenLevel;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.Heightmap;

import java.util.*;
import java.util.concurrent.ConcurrentHashMap;

/**
 * The carrier's cell sites along the fiber (Cellular v2 phase 5), laid by the fiber feature in
 * the chunk that holds them (a site's columns never cross a chunk border, {@link CellPlan}).
 * A site is built from the same primitives a player uses:
 *
 * <ul>
 *   <li>a Fiber Patch Panel against the chord's span at the fiber's height: a tap, exactly as a
 *       player taps the ring (the ring lists it as an attachment of its piece);</li>
 *   <li>a network cable riser from the panel to the computer on the ground (its UP face, eth1,
 *       from a fiber above; its DOWN face, eth0, from a fiber below);</li>
 *   <li>the computer (provisioned: address in the chord's /26, {@code celld-site} enabled);</li>
 *   <li>an Advanced SDR beside it, coax out one block and up {@link CellPlan#MAST} blocks to an
 *       LTE Sector Antenna facing along the chord, a lattice mast beside the coax.</li>
 * </ul>
 * The computer boots when its chunk loads, like a house computer; {@code celld-site start --wait}
 * waits for the SDR, which attaches once the chunk ticks.
 */
public final class CellSites {
    private CellSites() {}

    private static final Map<Long, Map<Long, List<CellPlan.SpanSite>>> BY_CHUNK = new ConcurrentHashMap<>();

    /** The span sites whose columns are in chunk (cx, cz). */
    public static List<CellPlan.SpanSite> inChunk(CellPlan plan, int cx, int cz) {
        var map = BY_CHUNK.computeIfAbsent(plan.seed, s -> {
            Map<Long, List<CellPlan.SpanSite>> m = new HashMap<>();
            for (CellPlan.SpanSite site : plan.allSpans())
                m.computeIfAbsent(FiberChords.chunk(site.x() >> 4, site.z() >> 4), k -> new ArrayList<>()).add(site);
            return m;
        });
        return map.getOrDefault(FiberChords.chunk(cx, cz), List.of());
    }

    /** Where a placed site's parts are (world positions), for checks and screenshots. */
    public record Layout(BlockPos computer, BlockPos sdr, BlockPos coaxBase, BlockPos antenna, BlockPos mastBase, BlockPos panel,
                         Direction screen, int eth) {}

    static Direction dir(int dataValue) {
        return Direction.from3DDataValue(dataValue);
    }

    /** First solid ground under the surface at (x, z): skips logs, leaves and plants. */
    private static int ground(WorldGenLevel level, int x, int z) {
        int y = level.getHeight(Heightmap.Types.MOTION_BLOCKING_NO_LEAVES, x, z);
        BlockPos.MutableBlockPos p = new BlockPos.MutableBlockPos(x, y - 1, z);
        while (p.getY() > level.getMinBuildHeight() + 1) {
            BlockState s = level.getBlockState(p);
            if (s.is(BlockTags.LOGS) || s.is(BlockTags.LEAVES) || s.canBeReplaced() && s.getFluidState().isEmpty()) p.move(Direction.DOWN);
            else break;
        }
        return p.getY() + 1;
    }

    private static boolean keep(BlockState s) {
        return s.is(Blocks.BEDROCK) || s.getBlock() instanceof FiberInfrastructureBlock || s.getBlock() instanceof TerminalBlock;
    }

    private static void set(WorldGenLevel level, BlockPos p, BlockState s) {
        if (!keep(level.getBlockState(p))) level.setBlock(p, s, 2);
    }

    /** Place one site (all its columns are in the chunk being decorated). */
    public static Layout place(WorldGenLevel level, CellPlan plan, CellPlan.SpanSite site) {
        Direction out = dir(site.out()), along = dir(site.along());
        int[][] cols = site.columns();
        BlockPos.MutableBlockPos m = new BlockPos.MutableBlockPos();
        int yc = Integer.MIN_VALUE;
        int[] groundAt = new int[cols.length];
        for (int i = 0; i < cols.length; i++) {
            groundAt[i] = ground(level, cols[i][0], cols[i][1]);
            yc = Math.max(yc, groundAt[i]);
        }
        int fy = site.panelY();
        boolean above = fy >= yc + 1;
        if (!above) yc = Math.max(yc, fy + 1);
        int top = yc + CellPlan.MAST;
        BlockState pad = Blocks.STONE_BRICKS.defaultBlockState(), air = Blocks.AIR.defaultBlockState();
        // A level pad at yc - 1, foundations down to the ground (or through water), the columns clear above.
        for (int i = 0; i < cols.length; i++) {
            int x = cols[i][0], z = cols[i][1];
            for (int y = Math.min(groundAt[i], yc) - 1; y < yc; y++) {
                if (i == 0 && !above && y >= fy) continue;   // the computer's own column holds the riser
                set(level, m.set(x, y, z), pad);
            }
            // Through water (or air over a lake), down to the floor.
            for (int y = Math.min(groundAt[i], yc) - 2; y > level.getMinBuildHeight(); y--) {
                BlockState below = level.getBlockState(m.set(x, y, z));
                if (below.getFluidState().isEmpty() && !below.canBeReplaced()) break;
                if (i == 0 && !above && y <= fy) break;
                set(level, m, pad);
            }
            for (int y = yc; y <= top + 1; y++) {
                if (i == 0 && above && y >= fy) break;
                BlockState s = level.getBlockState(m.set(x, y, z));
                if (!s.isAir()) set(level, m, air);
            }
        }
        BlockPos computer = new BlockPos(cols[0][0], yc, cols[0][1]);
        BlockPos sdr = new BlockPos(cols[1][0], yc, cols[1][1]);
        BlockPos coax = new BlockPos(cols[2][0], yc, cols[2][1]);
        BlockPos mast = new BlockPos(cols[3][0], yc, cols[3][1]);
        BlockPos panel = new BlockPos(site.x(), fy, site.z());
        Direction towardSpan = out.getOpposite();
        // The tap and the riser.
        BlockState cable = ModBlocks.NETWORK_CABLE.get().defaultBlockState();
        BlockState tap = ModBlocks.FIBER_PATCH_PANEL.get().defaultBlockState().setValue(FiberPatchPanelBlock.FACING, out)
                .setValue(NetworkCableBlock.getPropertyForDirection(towardSpan), true)
                .setValue(above ? NetworkCableBlock.DOWN : NetworkCableBlock.UP, true);
        level.setBlock(panel, tap, 2);
        int lo = above ? yc + 1 : fy + 1, hi = above ? fy - 1 : yc - 1;
        for (int y = lo; y <= hi; y++)
            level.setBlock(m.set(computer.getX(), y, computer.getZ()), cable.setValue(NetworkCableBlock.UP, true).setValue(NetworkCableBlock.DOWN, true), 2);
        // The computer, screen away from the SDR.
        Direction screen = along.getOpposite();
        level.setBlock(computer, ModBlocks.TERMINAL_BLOCK.get().defaultBlockState().setValue(TerminalBlock.FACING, screen), 2);
        // SDR (its front away from the computer: the coax jack is on its side), coax up, the panel.
        level.setBlock(sdr, RadioSdrContent.SDR_ADVANCED.get().defaultBlockState().setValue(SdrBlock.FACING, along), 2);
        BlockState coaxState = RadioAntennaContent.COAX_CABLE.get().defaultBlockState();
        for (int y = yc; y < top; y++) level.setBlock(m.set(coax.getX(), y, coax.getZ()), coaxState, 2);
        BlockPos antenna = new BlockPos(coax.getX(), top, coax.getZ());
        level.setBlock(antenna, CellularContent.LTE_ANTENNA.get().defaultBlockState().setValue(CellAntennaBlock.FACING, dir(site.facing())), 2);
        for (int y = yc; y < top; y++) {
            BlockPos p = new BlockPos(coax.getX(), y, coax.getZ());
            BlockState s = level.getBlockState(p);
            if (s.getBlock() instanceof ConductorBlock c) level.setBlock(p, c.connected(level, p, s), 2);
        }
        BlockState lattice = RadioAntennaContent.LATTICE_MAST.get().defaultBlockState();
        for (int y = yc; y <= top; y++)
            level.setBlock(m.set(mast.getX(), y, mast.getZ()), lattice.setValue(ConductorBlock.UP, y < top).setValue(ConductorBlock.DOWN, y > yc), 2);
        // Provision and attach the computer (as the house computers are).
        var server = level.getLevel();
        var data = WorldNetwork.get(server);
        data.plan(server);
        int eth = above ? 1 : 0;
        data.provisionSpan(server, site, eth);
        if (level.getBlockEntity(computer) instanceof TerminalBlockEntity be) {
            CompoundTag tag = new CompoundTag();
            tag.putUUID("computerId", data.identity(server, site.village(), site.role()));
            tag.putBoolean("wasRunning", true);
            tag.putString("wasmModule", "terminal_os.wasm");
            be.loadWithComponents(tag, level.registryAccess());
        }
        return new Layout(computer, sdr, coax, antenna, mast, panel, screen, eth);
    }

    /** Lay the sites in chunk (cx, cz); true if any. */
    public static boolean placeInChunk(WorldGenLevel level, CellPlan plan, int cx, int cz) {
        boolean placed = false;
        for (CellPlan.SpanSite site : inChunk(plan, cx, cz)) {
            long t0 = System.nanoTime();
            place(level, plan, site);
            PLACED.incrementAndGet();
            PLACE_NANOS.addAndGet(System.nanoTime() - t0);
            placed = true;
        }
        return placed;
    }

    /** Sites generated since the server started and the time spent placing and provisioning them (for the checks' log). */
    public static final java.util.concurrent.atomic.AtomicLong PLACED = new java.util.concurrent.atomic.AtomicLong(),
            PLACE_NANOS = new java.util.concurrent.atomic.AtomicLong();

    /**
     * Where a generated site's parts are, worked out again from the live world (for checks and
     * screenshots): the computer is the terminal in the site's column at or above its ground.
     */
    public static Layout locate(net.minecraft.server.level.ServerLevel level, CellPlan.SpanSite site) {
        int[][] cols = site.columns();
        Direction along = dir(site.along());
        BlockPos.MutableBlockPos m = new BlockPos.MutableBlockPos();
        for (int y = level.getMaxBuildHeight() - 1; y > level.getMinBuildHeight(); y--)
            if (level.getBlockState(m.set(cols[0][0], y, cols[0][1])).getBlock() instanceof TerminalBlock) {
                int yc = y;
                BlockPos coax = new BlockPos(cols[2][0], yc, cols[2][1]);
                boolean above = site.panelY() > yc;
                return new Layout(new BlockPos(cols[0][0], yc, cols[0][1]), new BlockPos(cols[1][0], yc, cols[1][1]), coax,
                        coax.above(CellPlan.MAST), new BlockPos(cols[3][0], yc, cols[3][1]), new BlockPos(site.x(), site.panelY(), site.z()),
                        along.getOpposite(), above ? 1 : 0);
            }
        return null;
    }
}
//?}
