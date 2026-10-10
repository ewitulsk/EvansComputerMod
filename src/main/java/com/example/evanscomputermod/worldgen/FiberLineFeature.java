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
        Map<Long, Integer> blocks = new HashMap<>();
        for (long e : entries) {
            int chord = (int) (e >> 32), index = (int) e;
            int[] p = ring.path(chord)[index];
            long packed = FiberChords.pack(p[0], p[1], p[2]);
            int mask = blocks.getOrDefault(packed, 0);
            for (int[] n : ring.neighbours(chord, index)) {
                Direction d = Direction.fromDelta(n[0] - p[0], n[1] - p[1], n[2] - p[2]);
                if (d != null) mask |= 1 << d.ordinal();
            }
            if (ring.isEndpoint(packed)) mask |= 1 << Direction.DOWN.ordinal();
            blocks.put(packed, mask);
        }
        BlockState span = ModBlocks.FIBER_SPAN.get().defaultBlockState();
        boolean placed = false;
        for (var b : blocks.entrySet()) {
            int x = FiberChords.unpackX(b.getKey()), y = FiberChords.unpackY(b.getKey()), z = FiberChords.unpackZ(b.getKey());
            if (y < level.getMinBuildHeight() || y >= level.getMaxBuildHeight()) continue;
            BlockPos pos = new BlockPos(x, y, z);
            if (!replaceable(level.getBlockState(pos))) continue;
            BlockState s = span;
            for (Direction d : Direction.values())
                s = s.setValue(NetworkCableBlock.getPropertyForDirection(d), (b.getValue() & (1 << d.ordinal())) != 0);
            level.setBlock(pos, s, 2);
            placed = true;
        }
        return placed;
    }

    /** Fiber carves through anything except bedrock and the ISP/house network itself. */
    public static boolean replaceable(BlockState s) {
        return !s.is(Blocks.BEDROCK) && !(s.getBlock() instanceof TerminalBlock) && !(s.getBlock() instanceof NetworkCableBlock)
                && !(s.getBlock() instanceof InterfaceBlock);
    }
}
//?}
