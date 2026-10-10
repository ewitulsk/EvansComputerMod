package com.example.evanscomputermod.worldgen;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.WorldNetwork;
import com.example.evanscomputermod.worldgen.mixin.SinglePoolElementAccessor;

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.level.block.Mirror;
import net.minecraft.world.level.levelgen.structure.BoundingBox;
import net.minecraft.world.level.levelgen.structure.PoolElementStructurePiece;
import net.minecraft.world.level.levelgen.structure.StructurePiece;
import net.minecraft.world.level.levelgen.structure.pools.SinglePoolElement;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplate;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplateManager;

import java.util.*;
import java.util.regex.Pattern;

/**
 * Chooses the networked houses of a freshly expanded jigsaw village and routes their
 * buried access cable back to the ISP. Deterministic for a world seed and village.
 *
 * <p>About half of the residential houses (at least two) get a router + PC desk. Each
 * house's cable runs under its floor, then under the streets (preferring road cells,
 * then other street ground, never under other buildings) to the ISP router. Later
 * routes reuse earlier cable, so the result is one tree: one shared access segment.
 *
 * <p>It also places the ISP's two Fiber Patch Panels on the mast top (at the positions
 * planned in {@code WorldNetwork}) and routes the ISP router's own cables for the
 * building's actual rotation ({@link IspCabling}): eth2/eth3 up the mast to the panels,
 * eth1 to the Data Center the ISP's east jigsaw attached.
 */
public final class VillageNetworkPlanner {
    private static final Pattern RESIDENTIAL = Pattern.compile(".*/houses/[a-z]+_(small|medium|big)_house_\\d+$");
    private static final Pattern NOT_A_BUILDING = Pattern.compile(".*(farm|pen|accessory|meeting_point|stable|pond|well|decor|lamp).*");
    private static final int ROAD = 4, STREET = 20, INSIDE = 4, REUSE = 1;

    private record Placed(PoolElementStructurePiece piece, ResourceLocation location, TemplateScan scan) {
        BlockPos world(BlockPos local) {
            return piece.getPosition().offset(StructureTemplate.transform(local, Mirror.NONE, piece.getRotation(), BlockPos.ZERO));
        }

        Direction world(Direction local) {
            return piece.getRotation().rotate(local);
        }
    }

    private static long key(int x, int z) {
        return BlockPos.asLong(x, 0, z);
    }

    public static Optional<TechNetworkPiece> plan(StructureTemplateManager templates, long seed, WorldNetwork.Village site,
                                                  List<StructurePiece> pieces) {
        List<Placed> placed = new ArrayList<>();
        for (StructurePiece p : pieces)
            if (p instanceof PoolElementStructurePiece pe && pe.getElement() instanceof SinglePoolElement single) {
                var loc = ((SinglePoolElementAccessor) single).ecm$template().left();
                if (loc.isEmpty()) continue;
                TemplateScan.of(templates, loc.get()).ifPresent(scan -> placed.add(new Placed(pe, loc.get(), scan)));
            }
        if (placed.isEmpty()) return Optional.empty();
        Placed isp = placed.get(0);
        BlockPos ispRouterLocal = isp.scan.findNbt("ecmRole", "isp.router");
        if (ispRouterLocal == null) return Optional.empty();
        BlockPos ispRouter = isp.world(ispRouterLocal);

        // The ISP's own cabling: panels on the planned mast sides, the router's runs.
        Placed datacenter = placed.stream().filter(p -> p.location.getPath().startsWith("tech_village/datacenter_"))
                .findFirst().orElse(null);
        boolean lan = datacenter != null
                && datacenter.world(new BlockPos(0, 4, 6)).equals(isp.world(IspCabling.DC_INLET));
        if (datacenter != null && !lan)
            com.example.evanscomputermod.EvansComputerMod.LOGGER.error(
                    "Tech Village {}: data center inlet {} is not beside the ISP conduit {}", site.number(),
                    datacenter.world(new BlockPos(0, 4, 6)), isp.world(IspCabling.DC_INLET));
        var cabling = IspCabling.plan(isp.scan, isp.piece.getPosition(), isp.piece.getRotation(), site.side(false),
                site.side(true), lan);
        List<TechNetworkPiece.Panel> panels = new ArrayList<>();
        long[] fiberEnds = new long[0];
        Map<Long, Integer> fixed = new HashMap<>();
        if (cabling == null) {
            com.example.evanscomputermod.EvansComputerMod.LOGGER.error("Tech Village {}: ISP cables do not fit", site.number());
        } else {
            for (boolean next : new boolean[] {false, true}) {
                BlockPos riserTop = (next ? cabling.nextRun() : cabling.prevRun()).get((next ? cabling.nextRun() : cabling.prevRun()).size() - 1);
                if (!riserTop.above().equals(site.panel(next)))
                    com.example.evanscomputermod.EvansComputerMod.LOGGER.error(
                            "Tech Village {}: {} riser ends under {}, planned panel {}", site.number(), next ? "next" : "prev",
                            riserTop.above(), site.panel(next));
                panels.add(new TechNetworkPiece.Panel(site.panel(next), site.side(next)));
            }
            fiberEnds = new long[] {cabling.prevExit().asLong(), site.panel(false).asLong(), cabling.nextExit().asLong(),
                    site.panel(true).asLong()};
            fixed.putAll(cabling.cables());
        }
        // The chat village's data center: the rack cable reaches down onto the chat server.
        if (datacenter != null && site.number() == WorldNetwork.chatVillage(seed)) {
            BlockPos c = datacenter.world(new BlockPos(11, 2, 4));
            int mask = 0;
            for (BlockPos q : List.of(datacenter.world(new BlockPos(11, 2, 3)), datacenter.world(new BlockPos(11, 2, 5)),
                    datacenter.world(new BlockPos(11, 1, 4)))) {
                Direction d = Direction.fromDelta(q.getX() - c.getX(), q.getY() - c.getY(), q.getZ() - c.getZ());
                if (d != null) mask |= 1 << d.ordinal();
            }
            fixed.put(c.asLong(), mask);
        }

        // Routable ground: road cells, other street ground, the ISP's footprint.
        Map<Long, Integer> cost = new HashMap<>();
        Map<Long, Integer> fixedY = new HashMap<>();
        BoundingBox ib = isp.piece.getBoundingBox();
        for (int x = ib.minX(); x <= ib.maxX(); x++)
            for (int z = ib.minZ(); z <= ib.maxZ(); z++) {
                cost.put(key(x, z), INSIDE);
                fixedY.put(key(x, z), ispRouter.getY() - 2);
            }
        Set<Long> blocked = new HashSet<>();
        List<Placed> houses = new ArrayList<>(), others = new ArrayList<>();
        for (Placed p : placed) {
            if (p == isp) continue;
            String path = p.location.getPath();
            BoundingBox b = p.piece.getBoundingBox();
            if (path.contains("/streets/")) {
                for (int x = b.minX(); x <= b.maxX(); x++)
                    for (int z = b.minZ(); z <= b.maxZ(); z++) cost.putIfAbsent(key(x, z), STREET);
                for (int[] c : p.scan.roadCells()) {
                    BlockPos w = p.world(new BlockPos(c[0], 0, c[1]));
                    cost.put(key(w.getX(), w.getZ()), ROAD);
                }
            } else if (path.contains("/houses/")) {
                for (int x = b.minX(); x <= b.maxX(); x++)
                    for (int z = b.minZ(); z <= b.maxZ(); z++) blocked.add(key(x, z));
                if (RESIDENTIAL.matcher(path).matches()) houses.add(p);
                else if (!NOT_A_BUILDING.matcher(path).matches()) others.add(p);
            }
        }
        for (long b : blocked) cost.remove(b);

        Comparator<Placed> order = Comparator.comparingInt((Placed p) -> p.piece.getPosition().getX())
                .thenComparingInt(p -> p.piece.getPosition().getZ()).thenComparingInt(p -> p.piece.getPosition().getY());
        houses.sort(order);
        others.sort(order);
        Random random = new Random(seed * 31 + site.number() * 0x9E3779B97F4A7C15L);
        Collections.shuffle(houses, random);
        Collections.shuffle(others, random);
        int target = Math.max(2, (houses.size() + 1) / 2);
        List<Placed> candidates = new ArrayList<>(houses);
        candidates.addAll(others);

        // Graph state
        Map<Long, Integer> index = new LinkedHashMap<>();
        List<int[]> nodes = new ArrayList<>(); // x, z, fixedY, dropTop
        Map<Integer, Set<Integer>> adjacency = new HashMap<>();
        List<TechNetworkPiece.Terminal> terminals = new ArrayList<>();
        Map<Long, Integer> lanCables = new HashMap<>(fixed);
        Set<Long> used = new HashSet<>();
        int ispNode = node(index, nodes, adjacency, ispRouter.getX(), ispRouter.getZ(), ispRouter.getY() - 2, ispRouter.getY() - 1);
        used.add(key(ispRouter.getX(), ispRouter.getZ()));

        int house = 0;
        for (Placed p : candidates) {
            if (house >= target) break;
            var desk = p.scan.desk();
            if (desk.isEmpty()) continue;
            BlockPos router = p.world(desk.get().router()), pc = p.world(desk.get().pc());
            Direction facing = p.world(desk.get().facing());
            int houseY = router.getY() - 2;
            BoundingBox hb = p.piece.getBoundingBox();
            List<Long> route = route(key(router.getX(), router.getZ()), key(ispRouter.getX(), ispRouter.getZ()), cost, hb, used);
            if (route == null) continue;
            house++;
            int prev = -1;
            for (int i = 0; i < route.size(); i++) {
                long c = route.get(i);
                int x = BlockPos.getX(c), z = BlockPos.getZ(c);
                int y = hb.isInside(x, hb.minY(), z) ? houseY : fixedY.getOrDefault(c, TechNetworkPiece.SURFACE);
                int drop = i == 0 ? router.getY() - 1 : TechNetworkPiece.SURFACE;
                int n = index.containsKey(c) ? index.get(c) : node(index, nodes, adjacency, x, z, y, drop);
                if (i == 0 && index.containsKey(c)) nodes.get(n)[3] = router.getY() - 1;
                if (prev >= 0) {
                    adjacency.get(prev).add(n);
                    adjacency.get(n).add(prev);
                }
                prev = n;
                used.add(c);
            }
            String role = "house" + house;
            terminals.add(new TechNetworkPiece.Terminal(role + ".router", router, facing));
            terminals.add(new TechNetworkPiece.Terminal(role + ".pc", pc, facing));
            Direction toPc = Direction.fromDelta(pc.getX() - router.getX(), 0, pc.getZ() - router.getZ());
            lanCables.put(router.above().asLong(), (1 << Direction.DOWN.ordinal()) | (1 << toPc.ordinal()));
            lanCables.put(pc.above().asLong(), (1 << Direction.DOWN.ordinal()) | (1 << toPc.getOpposite().ordinal()));
        }

        List<TechNetworkPiece.Node> out = new ArrayList<>();
        int minX = Integer.MAX_VALUE, minZ = Integer.MAX_VALUE, maxX = Integer.MIN_VALUE, maxZ = Integer.MIN_VALUE;
        for (int[] n : nodes) {
            out.add(new TechNetworkPiece.Node(n[0], n[1], n[2], n[3]));
            minX = Math.min(minX, n[0]);
            maxX = Math.max(maxX, n[0]);
            minZ = Math.min(minZ, n[1]);
            maxZ = Math.max(maxZ, n[1]);
        }
        List<BlockPos> extra = new ArrayList<>();
        for (var t : terminals) extra.add(t.pos());
        for (long c : lanCables.keySet()) extra.add(BlockPos.of(c));
        for (var p : panels) extra.add(p.pos());
        int y0 = ispRouter.getY(), minY = y0 - 40, maxY = y0 + 24;
        for (BlockPos p : extra) {
            minX = Math.min(minX, p.getX());
            maxX = Math.max(maxX, p.getX());
            minZ = Math.min(minZ, p.getZ());
            maxZ = Math.max(maxZ, p.getZ());
            minY = Math.min(minY, p.getY());
            maxY = Math.max(maxY, p.getY());
        }
        int[][] adj = new int[nodes.size()][];
        for (int i = 0; i < nodes.size(); i++) adj[i] = adjacency.get(i).stream().mapToInt(Integer::intValue).sorted().toArray();
        BoundingBox box = new BoundingBox(minX, minY, minZ, maxX, maxY, maxZ);
        return Optional.of(new TechNetworkPiece(site.number(), out, adj, terminals, lanCables, panels, fiberEnds, box));
    }

    private static int node(Map<Long, Integer> index, List<int[]> nodes, Map<Integer, Set<Integer>> adjacency,
                            int x, int z, int y, int drop) {
        int i = nodes.size();
        nodes.add(new int[] {x, z, y, drop});
        index.put(key(x, z), i);
        adjacency.put(i, new TreeSet<>());
        return i;
    }

    /** Cheapest 4-connected path from {@code from} (inside its house) to {@code to}. */
    private static List<Long> route(long from, long to, Map<Long, Integer> cost, BoundingBox house, Set<Long> used) {
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
            if (c == to || (used.contains(c) && c != from)) {
                List<Long> path = new ArrayList<>();
                for (Long at = c; at != null; at = back.get(at)) path.add(at);
                Collections.reverse(path);
                return path;
            }
            int x = BlockPos.getX(c), z = BlockPos.getZ(c);
            for (int[] s : steps) {
                int nx = x + s[0], nz = z + s[1];
                long n = key(nx, nz);
                int step;
                if (house.isInside(nx, house.minY(), nz)) step = INSIDE;
                else if (cost.containsKey(n)) step = used.contains(n) ? REUSE : cost.get(n);
                else continue;
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
