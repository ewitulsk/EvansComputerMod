package com.example.evanscomputermod.worldgen;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.*;
import com.example.evanscomputermod.computer.FiberChords;
import com.example.evanscomputermod.computer.WorldNetwork;

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.Holder;
import net.minecraft.data.worldgen.placement.PlacementUtils;
import net.minecraft.world.level.WorldGenLevel;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.feature.Feature;
import net.minecraft.world.level.levelgen.feature.FeaturePlaceContext;
import net.minecraft.world.level.levelgen.feature.configurations.NoneFeatureConfiguration;
import net.minecraft.world.level.levelgen.placement.PlacedFeature;

import java.util.HashMap;
import java.util.Map;

/**
 * Lays the long-distance fiber ring during world generation: in the chunk being
 * decorated it writes exactly the precomputed path blocks of every chord through that
 * chunk, replacing terrain, trees, water or buildings ("as if the villagers built it").
 * Connection properties come from the path itself, never from neighbouring chunks, so
 * the line is continuous however chunks generate. Runs at the last decoration step
 * (added to every overworld biome by a biome modifier, and to superflat worlds by
 * {@code FlatFiberFeatureMixin}).
 */
public final class FiberLineFeature extends Feature<NoneFeatureConfiguration> {
    private static volatile Holder<PlacedFeature> inline;

    public FiberLineFeature() {
        super(NoneFeatureConfiguration.CODEC);
    }

    /** A placed instance for generators that bypass biome modifiers (superflat). */
    public static Holder<PlacedFeature> inlinePlaced() {
        if (inline == null) inline = PlacementUtils.inlinePlaced(TechWorldgen.FIBER_LINE.get(), NoneFeatureConfiguration.INSTANCE);
        return inline;
    }

    @Override
    public boolean place(FeaturePlaceContext<NoneFeatureConfiguration> context) {
        WorldGenLevel level = context.level();
        FiberChords ring = WorldNetwork.FIBER.get(level.getSeed());
        if (ring == null || level.getLevel().dimension() != net.minecraft.world.level.Level.OVERWORLD) return false;
        int cx = context.origin().getX() >> 4, cz = context.origin().getZ() >> 4;
        long[] entries = ring.inChunk(cx, cz);
        if (entries.length == 0) return false;
        BlockState span = ModBlocks.FIBER_SPAN.get().defaultBlockState();
        boolean placed = false;
        java.util.Set<Long> outside = new java.util.HashSet<>();
        for (long e : entries) {
            int chord = (int) (e >> 32), index = (int) e;
            int[] p = ring.path(chord)[index];
            placed |= place(level, ring, FiberChords.pack(p[0], p[1], p[2]), span, false);
            for (int[] n : ring.neighbours(chord, index))
                if ((n[0] >> 4) != cx || (n[2] >> 4) != cz) outside.add(FiberChords.pack(n[0], n[1], n[2]));
        }
        // Path neighbours in adjacent chunks whose fiber already exists: re-assert their
        // canonical state, in case a shape update from this chunk's own generation (made
        // before the fiber replaced a block here) dropped the arm toward this chunk.
        for (long n : outside) place(level, ring, n, span, true);
        return placed;
    }

    /**
     * Put the ring's path blocks back in one chunk of a live world (each placement
     * repairs a recorded break). Used after placing a village straight into a world whose
     * fiber already exists ({@code /place}-style, GameTests): that placement snaps
     * terrain-matching streets to the live surface heightmap, which counts an overhead
     * fiber line as the surface. Natural generation places structures before this feature.
     */
    public static int reassert(net.minecraft.server.level.ServerLevel level, FiberChords ring, int chunkX, int chunkZ) {
        BlockState span = ModBlocks.FIBER_SPAN.get().defaultBlockState();
        int placed = 0;
        for (long e : ring.inChunk(chunkX, chunkZ)) {
            int[] p = ring.path((int) (e >> 32))[(int) e];
            BlockPos pos = new BlockPos(p[0], p[1], p[2]);
            if (level.getBlockState(pos).is(span.getBlock()) || !replaceable(level.getBlockState(pos))) continue;
            int arms = ring.arms(FiberChords.pack(p[0], p[1], p[2]));
            BlockState s = span;
            for (Direction d : Direction.values())
                s = s.setValue(NetworkCableBlock.getPropertyForDirection(d), (arms & (1 << d.ordinal())) != 0);
            level.setBlock(pos, s, 3);
            placed++;
        }
        return placed;
    }

    private static boolean place(WorldGenLevel level, FiberChords ring, long packed, BlockState span, boolean onlyIfFiber) {
        int x = FiberChords.unpackX(packed), y = FiberChords.unpackY(packed), z = FiberChords.unpackZ(packed);
        if (y < level.getMinBuildHeight() || y >= level.getMaxBuildHeight()) return false;
        BlockPos pos = new BlockPos(x, y, z);
        BlockState old = level.getBlockState(pos);
        if (onlyIfFiber ? !old.is(span.getBlock()) : !replaceable(old)) return false;
        int arms = ring.arms(packed);
        BlockState s = span;
        for (Direction d : Direction.values())
            s = s.setValue(NetworkCableBlock.getPropertyForDirection(d), (arms & (1 << d.ordinal())) != 0);
        if (s == old) return false;
        level.setBlock(pos, s, 2);
        return true;
    }

    /** Fiber carves through anything except bedrock and the ISP/house network itself. */
    public static boolean replaceable(BlockState s) {
        return !s.is(Blocks.BEDROCK) && !(s.getBlock() instanceof TerminalBlock) && !(s.getBlock() instanceof NetworkCableBlock)
                && !(s.getBlock() instanceof InterfaceBlock);
    }
}
//?}
