package com.example.evanscomputermod.testing;

//? if <=1.21.1 {
import com.example.evanscomputermod.worldgen.VillageRoadPlanner;

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.levelgen.structure.StructureStart;

import java.util.*;

/**
 * Reads a generated Tech Village and checks that every building is on the road network: from
 * each marked entrance a player can walk on road blocks (stepping up or down one block, two
 * blocks of headroom) to one of the village's vanilla streets. It only reads blocks; the roads
 * come from world generation ({@link VillageRoadPlanner}).
 */
public final class TechRoadChecks {
    private TechRoadChecks() {}

    private static final int LIMIT = 40_000;

    /** Null when every entrance reaches a street, else what is wrong; {@code details} gets one line per entrance. */
    public static String check(ServerLevel level, StructureStart start, List<String> details) {
        var placed = VillageRoadPlanner.placed(level.getStructureManager(), start.getPieces());
        var unmarked = VillageRoadPlanner.unmarked(placed);
        if (!unmarked.isEmpty()) return "buildings without an entrance marker (no road): " + unmarked;
        var entrances = VillageRoadPlanner.entrances(placed);
        if (entrances.isEmpty()) return "the village has no building entrances";
        Set<Long> streets = VillageRoadPlanner.streetRoads(placed);
        if (streets.isEmpty()) return "the village has no streets";
        long roadCells = start.getPieces().stream().filter(p -> p instanceof com.example.evanscomputermod.worldgen.TechRoadPiece)
                .mapToLong(p -> ((com.example.evanscomputermod.worldgen.TechRoadPiece) p).cells().length).sum();
        details.add("planned road cells " + roadCells);
        List<String> missing = new ArrayList<>();
        for (var e : entrances) {
            int steps = walk(level, e.pos(), streets);
            String name = e.building().substring(e.building().lastIndexOf('/') + 1);
            details.add(name + " door " + e.pos().toShortString() + (steps < 0 ? ": no road to a street (" + around(level, e.pos()) + ")"
                    : ": street in " + steps + " blocks"));
            if (steps < 0) missing.add(name + " at " + e.pos().toShortString());
        }
        if (!missing.isEmpty()) details.add("unpaved planned cells: " + unpaved(level, start, entrances.get(0).pos().getY()));
        return missing.isEmpty() ? null : "no road from the entrance to a street: " + String.join(", ", missing);
    }

    /** Planned road cells with no road block near the village's ground level, and what is on top there. */
    private static String unpaved(ServerLevel level, StructureStart start, int y) {
        List<String> out = new ArrayList<>();
        for (var p : start.getPieces())
            if (p instanceof com.example.evanscomputermod.worldgen.TechRoadPiece road)
                for (long c : road.cells()) {
                    int x = BlockPos.getX(c), z = BlockPos.getZ(c);
                    boolean ok = false;
                    for (int yy = y - 6; yy <= y + 6 && !ok; yy++) ok = VillageRoadPlanner.isRoad(level.getBlockState(new BlockPos(x, yy, z)));
                    if (!ok) {
                        int top = level.getHeight(net.minecraft.world.level.levelgen.Heightmap.Types.WORLD_SURFACE, x, z) - 1;
                        out.add(x + "," + z + "=" + name(level, new BlockPos(x, top, z)) + "@" + top);
                    }
                }
        return out.size() > 20 ? out.subList(0, 20) + " (+" + (out.size() - 20) + ")" : out.toString();
    }

    /** What a player standing at {@code p} sees: the block, what is above it, its neighbours. */
    private static String around(ServerLevel level, BlockPos p) {
        StringBuilder b = new StringBuilder(name(level, p) + " under " + name(level, p.above()) + "/" + name(level, p.above(2)));
        for (Direction d : Direction.Plane.HORIZONTAL)
            b.append(", ").append(d.getSerializedName()).append(' ').append(name(level, p.relative(d)))
                    .append(" under ").append(name(level, p.relative(d).above()));
        return b.toString();
    }

    private static String name(ServerLevel level, BlockPos p) {
        return net.minecraft.core.registries.BuiltInRegistries.BLOCK.getKey(level.getBlockState(p).getBlock()).getPath();
    }

    /** Road blocks walked from {@code from} to the first street road cell, or -1. */
    public static int walk(ServerLevel level, BlockPos from, Set<Long> streets) {
        if (!standable(level, from)) return -1;
        Map<BlockPos, Integer> seen = new HashMap<>(Map.of(from, 0));
        Deque<BlockPos> queue = new ArrayDeque<>(List.of(from));
        while (!queue.isEmpty() && seen.size() < LIMIT) {
            BlockPos p = queue.poll();
            int d = seen.get(p);
            if (streets.contains(BlockPos.asLong(p.getX(), 0, p.getZ()))) return d;
            for (Direction dir : Direction.Plane.HORIZONTAL)
                for (int dy = -1; dy <= 1; dy++) {
                    BlockPos q = p.relative(dir).above(dy);
                    if (!seen.containsKey(q) && standable(level, q)) {
                        seen.put(q, d + 1);
                        queue.add(q);
                    }
                }
        }
        return -1;
    }

    private static boolean standable(ServerLevel level, BlockPos p) {
        return VillageRoadPlanner.isRoad(level.getBlockState(p))
                && level.getBlockState(p.above()).getCollisionShape(level, p.above()).isEmpty()
                && level.getBlockState(p.above(2)).getCollisionShape(level, p.above(2)).isEmpty();
    }
}
//?}
