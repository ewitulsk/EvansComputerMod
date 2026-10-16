package com.example.evanscomputermod.worldgen;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.worldgen.mixin.SinglePoolElementAccessor;

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.tags.BlockTags;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.Mirror;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.structure.BoundingBox;
import net.minecraft.world.level.levelgen.structure.PoolElementStructurePiece;
import net.minecraft.world.level.levelgen.structure.StructurePiece;
import net.minecraft.world.level.levelgen.structure.pools.SinglePoolElement;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplate;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplateManager;

import java.util.*;

/**
 * Puts every Tech Village building on the village's road network. A building template marks
 * the ground cell outside each door with an {@link #ENTRANCE} jigsaw (scripts/gen-tech-village.py
 * adds one per door and refuses a building without one); after the jigsaw village expands, this
 * paves the cheapest road from each entrance to the nearest vanilla street, around every other
 * piece's walls and furniture, as one {@link TechRoadPiece}. Nothing here names the ISP, the Data
 * Center or the Phone Store: a building added later only needs its entrance marker.
 */
public final class VillageRoadPlanner {
    /** Jigsaw name of a building entrance; it never attaches and turns into the style's path block. */
    public static final String ENTRANCE = "evanscomputermod:entrance";
    /** Search margin around the village's pieces, blocks. */
    private static final int MARGIN = 16;
    private static final int ROAD = 1, OPEN = 3, YARD = 5;

    private VillageRoadPlanner() {}

    /** A placed jigsaw piece with a readable template. */
    public record Placed(PoolElementStructurePiece piece, ResourceLocation location, TemplateScan scan) {
        public BlockPos world(BlockPos local) {
            return piece.getPosition().offset(StructureTemplate.transform(local, Mirror.NONE, piece.getRotation(), BlockPos.ZERO));
        }

        public boolean street() {
            return location.getPath().contains("/streets/");
        }
    }

    /** A building's entrance in world coordinates. */
    public record Entrance(String building, BlockPos pos, Direction out) {}

    public static List<Placed> placed(StructureTemplateManager templates, List<StructurePiece> pieces) {
        List<Placed> out = new ArrayList<>();
        for (StructurePiece p : pieces)
            if (p instanceof PoolElementStructurePiece pe && pe.getElement() instanceof SinglePoolElement single) {
                var loc = ((SinglePoolElementAccessor) single).ecm$template().left();
                if (loc.isEmpty()) continue;
                TemplateScan.of(templates, loc.get()).ifPresent(scan -> out.add(new Placed(pe, loc.get(), scan)));
            }
        return out;
    }

    /** Every marked entrance of the village's buildings, in world coordinates. */
    public static List<Entrance> entrances(List<Placed> placed) {
        List<Entrance> out = new ArrayList<>();
        for (Placed p : placed)
            for (var e : p.scan().entrances())
                out.add(new Entrance(p.location().getPath(), p.world(e.pos()), p.piece().getRotation().rotate(e.out())));
        return out;
    }

    /** Mod buildings with a door but no entrance marker: they would get no road. */
    public static List<String> unmarked(List<Placed> placed) {
        return placed.stream().filter(p -> p.location().getNamespace().equals(EvansComputerMod.MODID)
                        && p.scan().hasDoor() && p.scan().entrances().isEmpty())
                .map(p -> p.location().toString()).distinct().toList();
    }

    /** World (x, z) cells of the vanilla streets' road surface. */
    public static Set<Long> streetRoads(List<Placed> placed) {
        Set<Long> out = new HashSet<>();
        for (Placed p : placed)
            if (p.street())
                for (int[] c : p.scan().roadCells()) {
                    BlockPos w = p.world(new BlockPos(c[0], 0, c[1]));
                    out.add(key(w.getX(), w.getZ()));
                }
        return out;
    }

    /** Blocks a village road is made of (street surfaces, our paths, plank bridges). */
    public static boolean isRoad(BlockState s) {
        return s.is(Blocks.DIRT_PATH) || s.is(Blocks.SMOOTH_SANDSTONE) || s.is(Blocks.TERRACOTTA) || s.is(BlockTags.PLANKS);
    }

    static long key(int x, int z) {
        return BlockPos.asLong(x, 0, z);
    }

    public static Optional<TechRoadPiece> plan(StructureTemplateManager templates, int village, List<StructurePiece> pieces) {
        List<Placed> placed = placed(templates, pieces);
        for (String l : unmarked(placed))
            EvansComputerMod.LOGGER.error("Tech Village {}: building {} has doors but no {} jigsaw; it gets no road", village, l, ENTRANCE);
        List<Entrance> entrances = entrances(placed);
        if (entrances.isEmpty()) return Optional.empty();

        // Grid: blocked columns (other pieces' walls and furniture), cheap cells (paths already
        // in the templates), yards (open ground inside a building's box), the streets' road.
        Set<Long> targets = streetRoads(placed);
        Set<Long> blocked = new HashSet<>();
        Set<Long> cheap = new HashSet<>();
        Set<Long> yard = new HashSet<>();
        int minX = Integer.MAX_VALUE, minZ = Integer.MAX_VALUE, maxX = Integer.MIN_VALUE, maxZ = Integer.MIN_VALUE;
        Map<BlockState, Integer> surfaces = new HashMap<>();
        for (Placed p : placed) {
            BoundingBox b = p.piece().getBoundingBox();
            minX = Math.min(minX, b.minX());
            minZ = Math.min(minZ, b.minZ());
            maxX = Math.max(maxX, b.maxX());
            maxZ = Math.max(maxZ, b.maxZ());
            if (p.street()) {
                for (int[] c : p.scan().roadCells()) surfaces.merge(p.scan().state(c[0], 0, c[1]), 1, Integer::sum);
                continue;
            }
            for (int x = b.minX(); x <= b.maxX(); x++)
                for (int z = b.minZ(); z <= b.maxZ(); z++) yard.add(key(x, z));
            for (long c : p.scan().footprint()) {
                BlockPos w = p.world(new BlockPos(BlockPos.getX(c), 0, BlockPos.getZ(c)));
                blocked.add(key(w.getX(), w.getZ()));
            }
            for (int[] c : p.scan().roadCells()) {
                BlockPos w = p.world(new BlockPos(c[0], 0, c[1]));
                cheap.add(key(w.getX(), w.getZ()));
            }
        }
        targets.removeAll(blocked);
        BoundingBox area = new BoundingBox(minX - MARGIN, 0, minZ - MARGIN, maxX + MARGIN, 0, maxZ + MARGIN);
        // The village's own road surface: the streets' most common road block.
        BlockState surface = surfaces.entrySet().stream()
                .max(Map.Entry.<BlockState, Integer>comparingByValue().thenComparing(e -> e.getKey().toString()))
                .map(Map.Entry::getKey).orElse(Blocks.DIRT_PATH.defaultBlockState());

        Set<Long> line = new LinkedHashSet<>();
        for (Entrance e : entrances) {
            List<Long> route = route(key(e.pos().getX(), e.pos().getZ()), targets, line, blocked, cheap, yard, area);
            if (route == null) {
                EvansComputerMod.LOGGER.error("Tech Village {}: no road from {} entrance {} to a street", village, e.building(),
                        e.pos().toShortString());
                continue;
            }
            line.addAll(route);
        }
        if (line.isEmpty()) return Optional.empty();
        // Three wide, like the streets: the centre line and its open neighbours.
        Set<Long> cells = new LinkedHashSet<>(line);
        for (long c : line)
            for (int dx = -1; dx <= 1; dx++)
                for (int dz = -1; dz <= 1; dz++) {
                    long n = key(BlockPos.getX(c) + dx, BlockPos.getZ(c) + dz);
                    if (!blocked.contains(n)) cells.add(n);
                }
        int bx0 = Integer.MAX_VALUE, bz0 = Integer.MAX_VALUE, bx1 = Integer.MIN_VALUE, bz1 = Integer.MIN_VALUE;
        for (long c : cells) {
            bx0 = Math.min(bx0, BlockPos.getX(c));
            bz0 = Math.min(bz0, BlockPos.getZ(c));
            bx1 = Math.max(bx1, BlockPos.getX(c));
            bz1 = Math.max(bz1, BlockPos.getZ(c));
        }
        int y = entrances.get(0).pos().getY();
        BoundingBox box = new BoundingBox(bx0, y - 48, bz0, bx1, y + 48, bz1);
        return Optional.of(new TechRoadPiece(cells.stream().mapToLong(Long::longValue).toArray(), surface, box));
    }

    /** Cheapest 4-connected cells from {@code from} to a street road cell or a road already planned. */
    private static List<Long> route(long from, Set<Long> streets, Set<Long> planned, Set<Long> blocked, Set<Long> cheap,
                                    Set<Long> yard, BoundingBox area) {
        Map<Long, Integer> dist = new HashMap<>();
        Map<Long, Long> back = new HashMap<>();
        PriorityQueue<long[]> queue = new PriorityQueue<>(Comparator.<long[]>comparingLong(a -> a[0]).thenComparingLong(a -> a[1]));
        dist.put(from, 0);
        queue.add(new long[] {0, from});
        int[][] steps = {{0, -1}, {0, 1}, {-1, 0}, {1, 0}};
        while (!queue.isEmpty()) {
            long[] head = queue.poll();
            long c = head[1];
            if (head[0] > dist.getOrDefault(c, Integer.MAX_VALUE)) continue;
            if (streets.contains(c) || (planned.contains(c) && c != from)) {
                List<Long> path = new ArrayList<>();
                for (Long at = c; at != null; at = back.get(at)) path.add(at);
                Collections.reverse(path);
                return path;
            }
            int x = BlockPos.getX(c), z = BlockPos.getZ(c);
            for (int[] s : steps) {
                int nx = x + s[0], nz = z + s[1];
                long n = key(nx, nz);
                if (blocked.contains(n) || !area.isInside(nx, 0, nz)) continue;
                int step = cheap.contains(n) || planned.contains(n) ? ROAD : yard.contains(n) ? YARD : OPEN;
                int d = dist.get(c) + step;
                if (d < dist.getOrDefault(n, Integer.MAX_VALUE)) {
                    dist.put(n, d);
                    back.put(n, c);
                    queue.add(new long[] {d, n});
                }
            }
        }
        return null;
    }
}
//?}
