package com.example.evanscomputermod.radio.conductor;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.antenna.graph.CoaxSpec;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.state.BlockState;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * A traced feedline: the coax/hardline run from a start block (usually a
 * feed point's coax port) to whatever it plugs into. Amplifier, tuner and
 * hazard lanes read loss, rating and the arrestor flag from here.
 *
 * @param blocks     the feedline blocks in order from the start
 * @param counts     blocks per tier
 * @param arrestor   true if a lightning arrestor is in the run
 * @param end        the non-feedline block the run plugs into (null if it ends open)
 * @param branched   true if the run forks (only the first branch is followed)
 */
public record Feedline(List<BlockPos> blocks, Map<CoaxSpec, Integer> counts, boolean arrestor,
        @Nullable BlockPos end, boolean branched) {
    public static final int MAX_BLOCKS = 512;

    public int length() { return blocks.size(); }

    /** Total matched-line loss at a frequency, dB. */
    public double lossDb(double hz) {
        double sum = 0;
        for (var e : counts.entrySet()) sum += e.getKey().lossDb(hz, e.getValue());
        return sum;
    }

    /** Lowest power rating along the run at a frequency, W (∞ for an empty run). */
    public double powerRatingW(double hz) {
        double min = Double.POSITIVE_INFINITY;
        for (CoaxSpec s : counts.keySet()) min = Math.min(min, s.powerRatingW(hz));
        return min;
    }

    /**
     * Follows the feedline starting next to {@code from} through its coax
     * port sides. Returns an empty run if nothing is connected.
     */
    public static Feedline trace(Level level, BlockPos from) {
        List<BlockPos> path = new ArrayList<>();
        Map<CoaxSpec, Integer> counts = new LinkedHashMap<>();
        Set<BlockPos> seen = new HashSet<>();
        seen.add(from);
        boolean arrestor = false, branched = false;
        BlockPos end = null;
        BlockPos cur = next(level, from, seen);
        while (cur != null && path.size() < MAX_BLOCKS) {
            BlockState s = level.getBlockState(cur);
            if (!(s.getBlock() instanceof CoaxBlock coax)) {
                end = cur;
                break;
            }
            path.add(cur);
            seen.add(cur);
            counts.merge(RadioConductors.coax(s), 1, Integer::sum);
            arrestor |= coax.grounded();
            List<BlockPos> outs = outs(level, cur, seen);
            if (outs.size() > 1) branched = true;
            cur = outs.isEmpty() ? null : outs.get(0);
        }
        return new Feedline(List.copyOf(path), Map.copyOf(counts), arrestor, end, branched);
    }

    @Nullable
    private static BlockPos next(Level level, BlockPos pos, Set<BlockPos> seen) {
        List<BlockPos> o = outs(level, pos, seen);
        return o.isEmpty() ? null : o.get(0);
    }

    /** Connected coax-kind sides of {@code pos} not visited yet. */
    private static List<BlockPos> outs(Level level, BlockPos pos, Set<BlockPos> seen) {
        BlockState s = level.getBlockState(pos);
        List<BlockPos> out = new ArrayList<>();
        if (!(s.getBlock() instanceof ConductorBlock c)) return out;
        for (Direction d : Direction.values()) {
            if (c.sideKind(s, d) != ConductorBlock.SideKind.COAX || !s.getValue(ConductorBlock.property(d))) continue;
            BlockPos n = pos.relative(d);
            if (!seen.contains(n)) out.add(n);
        }
        return out;
    }
}
//?}
