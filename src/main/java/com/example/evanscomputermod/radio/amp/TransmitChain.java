package com.example.evanscomputermod.radio.amp;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.antenna.graph.CoaxSpec;
import com.example.evanscomputermod.radio.conductor.CoaxBlock;
import com.example.evanscomputermod.radio.conductor.ConductorBlock;
import com.example.evanscomputermod.radio.conductor.FeedPointBlock;
import com.example.evanscomputermod.radio.conductor.Feedline;
import com.example.evanscomputermod.radio.conductor.RadioConductors;
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
 * The signal chain from an exciter (an SDR) to an antenna, as built in the
 * world (spec, Power: signal chain):
 * <pre>exciter ─ coax ─ [amplifier] ─ coax ─ [tuner] ─ coax (arrestor) ─ feed point</pre>
 * Each {@link Hop} is a (possibly empty) feedline run followed by the device
 * it plugs into. Devices connect either by touching (an SDR next to an
 * amplifier) or through coax / hardline runs; coax joins amplifiers and
 * tuners through {@code #evanscomputermod:rf_coax_ports}. Ports have no
 * direction: the chain leaves each device through whichever other side
 * leads on. The chain ends at a feed point (its coax port), or open.
 *
 * <p>Immutable; equal chains compare equal, so a holder can tell when the
 * build changed.
 */
public record TransmitChain(BlockPos exciter, List<Hop> hops) {
    public static final int MAX_HOPS = 8;

    public enum Kind { AMPLIFIER, TUNER, FEED, OPEN }

    /** A feedline run ({@code line}, maybe empty) into a device of {@code kind} at {@code pos} (null when OPEN at nothing). */
    public record Hop(Feedline line, Kind kind, @Nullable BlockPos pos) {}

    public static final TransmitChain NONE = new TransmitChain(BlockPos.ZERO, List.of());

    /** True if anything (coax, amplifier, tuner, feed point) is attached to the exciter. */
    public boolean connected() {
        return !hops.isEmpty();
    }

    /** The feed point at the end, or null if the chain ends open. */
    @Nullable
    public BlockPos feed() {
        if (hops.isEmpty()) return null;
        Hop last = hops.get(hops.size() - 1);
        return last.kind == Kind.FEED ? last.pos : null;
    }

    /** Index of the first amplifier hop, or -1. */
    public int ampIndex() {
        for (int i = 0; i < hops.size(); i++) if (hops.get(i).kind == Kind.AMPLIFIER) return i;
        return -1;
    }

    /** Index of the first tuner hop, or -1. */
    public int tunerIndex() {
        for (int i = 0; i < hops.size(); i++) if (hops.get(i).kind == Kind.TUNER) return i;
        return -1;
    }

    /** Matched loss of every feedline run, dB. */
    public double totalLineLossDb(double hz) {
        double s = 0;
        for (Hop h : hops) s += h.line.lossDb(hz);
        return s;
    }

    /** Loss of the runs up to and including hop {@code index}'s run, dB. */
    public double lossThroughDb(int index, double hz) {
        double s = 0;
        for (int i = 0; i <= index && i < hops.size(); i++) s += hops.get(i).line.lossDb(hz);
        return s;
    }

    /** True if any run has a lightning arrestor. */
    public boolean hasArrestor() {
        for (Hop h : hops) if (h.line.arrestor()) return true;
        return false;
    }

    /** Every feedline block in the chain. */
    public List<BlockPos> lineBlocks() {
        List<BlockPos> all = new ArrayList<>();
        for (Hop h : hops) all.addAll(h.line.blocks());
        return all;
    }

    // ------------------------------------------------------------ resolving

    /** Follows the chain from the exciter at {@code exciter}. */
    public static TransmitChain resolve(Level level, BlockPos exciter) {
        Set<BlockPos> seen = new HashSet<>();
        seen.add(exciter);
        List<Hop> hops = new ArrayList<>();
        BlockPos cur = exciter;
        for (int i = 0; i < MAX_HOPS; i++) {
            Hop h = next(level, cur, seen);
            if (h == null) break;
            hops.add(h);
            if (h.kind == Kind.FEED || h.kind == Kind.OPEN) break;
            cur = h.pos;
        }
        return new TransmitChain(exciter.immutable(), List.copyOf(hops));
    }

    @Nullable
    private static Kind deviceKind(BlockState s, Direction from) {
        if (s.getBlock() instanceof AmplifierBlock) return Kind.AMPLIFIER;
        if (s.getBlock() instanceof TunerBlock) return Kind.TUNER;
        if (s.getBlock() instanceof FeedPointBlock f && f.sideKind(s, from) == ConductorBlock.SideKind.COAX) return Kind.FEED;
        return null;
    }

    /** The next hop out of the device at {@code pos}: a touching device first, else a coax run. */
    @Nullable
    private static Hop next(Level level, BlockPos pos, Set<BlockPos> seen) {
        for (Direction d : Direction.values()) {
            BlockPos n = pos.relative(d);
            if (seen.contains(n)) continue;
            Kind k = deviceKind(level.getBlockState(n), d.getOpposite());
            if (k != null) {
                seen.add(n);
                return new Hop(empty(), k, n.immutable());
            }
        }
        for (Direction d : Direction.values()) {
            BlockPos n = pos.relative(d);
            if (seen.contains(n)) continue;
            BlockState s = level.getBlockState(n);
            if (s.getBlock() instanceof CoaxBlock && s.getValue(ConductorBlock.property(d.getOpposite())))
                return run(level, n, seen);
        }
        return null;
    }

    private static Feedline empty() {
        return new Feedline(List.of(), Map.of(), false, null, false);
    }

    /** Traces a coax run starting at {@code start} to whatever it plugs into. */
    private static Hop run(Level level, BlockPos start, Set<BlockPos> seen) {
        List<BlockPos> path = new ArrayList<>();
        Map<CoaxSpec, Integer> counts = new LinkedHashMap<>();
        boolean arrestor = false, branched = false;
        BlockPos cur = start.immutable();
        while (path.size() < Feedline.MAX_BLOCKS) {
            BlockState s = level.getBlockState(cur);
            path.add(cur);
            seen.add(cur);
            counts.merge(RadioConductors.coax(s), 1, Integer::sum);
            arrestor |= ((CoaxBlock) s.getBlock()).grounded();
            BlockPos nextCoax = null, device = null;
            Kind deviceKind = null;
            int outs = 0;
            for (Direction d : Direction.values()) {
                if (!s.getValue(ConductorBlock.property(d))) continue;
                BlockPos n = cur.relative(d);
                if (seen.contains(n)) continue;
                BlockState ns = level.getBlockState(n);
                outs++;
                if (ns.getBlock() instanceof CoaxBlock) {
                    if (nextCoax == null) nextCoax = n.immutable();
                } else if (device == null) {
                    Kind k = deviceKind(ns, d.getOpposite());
                    if (k != null) {
                        device = n.immutable();
                        deviceKind = k;
                    }
                }
            }
            if (outs > 1) branched = true;
            if (device != null) {
                seen.add(device);
                return new Hop(new Feedline(List.copyOf(path), Map.copyOf(counts), arrestor, device, branched), deviceKind, device);
            }
            if (nextCoax == null) break;
            cur = nextCoax;
        }
        return new Hop(new Feedline(List.copyOf(path), Map.copyOf(counts), arrestor, null, branched), Kind.OPEN, null);
    }
}
//?}
