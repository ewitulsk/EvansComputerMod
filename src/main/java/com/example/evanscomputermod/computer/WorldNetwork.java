package com.example.evanscomputermod.computer;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;

import net.minecraft.core.*;
import net.minecraft.nbt.*;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.tags.BiomeTags;
import net.minecraft.tags.TagKey;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.level.saveddata.SavedData;

import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.*;

/**
 * Durable village identities, the long-distance fiber ring and last-known cable blocks.
 *
 * <p>Village sites, their style and their fiber endpoints are fixed at plan time, before
 * any village chunk exists: the ISP mast stands on the site's (x, z) and the start
 * piece's height comes from the generator's {@code WORLD_SURFACE_WG} estimate, exactly
 * as JigsawStructure computes it. Each mast top carries two Fiber Patch Panels on
 * world-fixed sides (one toward the previous village, one toward the next), so the panel
 * and endpoint positions do not depend on the building's random rotation. The fiber
 * paths between neighbouring endpoints are derived from those endpoints
 * ({@link FiberChords}).
 *
 * <p>The fiber between two ISP routers is physical: router eth3, the cable up the mast,
 * the patch panel, the chord's Fiber Span, the neighbour's panel, its cable and eth2 form
 * one cable segment ({@link RingPieces}, {@link SegmentGraph}). Breaking a span splits
 * the chord into two pieces; an admin cut is a virtual break at the chord's midpoint; a
 * patch panel placed against the path joins the piece it touches (a tap). Before a
 * village has been seen loaded its fiber ports stand in as plugged into their chord
 * ends (and its data center as cabled), so the ring works headless from world creation;
 * afterwards the real cables decide, read from the last-known topology when unloaded.
 */
public final class WorldNetwork extends SavedData implements CableNetworkManager.Topology {
    public static final List<String> STYLES = List.of("plains", "desert", "savanna", "snowy", "taiga");
    /** ISP router NICs: DOWN/UP faces are eth0/eth1 for any horizontal facing. */
    public static final int ACCESS_PORT = 0, SERVER_PORT = 1, FIBER_PREV_PORT = 2, FIBER_NEXT_PORT = 3, UPLINK_PORT = 4;
    /** Data center servers use their UP face (eth1) on the server LAN. */
    public static final int SERVER_NIC = 1;
    public static final String ISP_ROUTER = "isp.router", WEB = "datacenter.web", CHAT = "datacenter.chat";
    public static final int CHAT_PORT = 7777;

    /**
     * A ring site: AS 65000 + number, vanilla style, start height, fiber endpoint height
     * (one above the mast top) and the mast sides (Direction 3D data values) of the panels
     * toward the previous and the next village.
     */
    public record Village(int number, int x, int z, String style, int groundY, int endY, int prevSide, int nextSide) {
        public Direction side(boolean next) {
            return Direction.from3DDataValue(next ? nextSide : prevSide);
        }

        /** The top lattice block of the mast. */
        public BlockPos mastTop() {
            return new BlockPos(x, endY - 1, z);
        }

        /** The Fiber Patch Panel toward the next ({@code true}) or previous village. */
        public BlockPos panel(boolean next) {
            return mastTop().relative(side(next));
        }

        /** The first fiber block of that direction's chord, on top of the panel. */
        public BlockPos endpoint(boolean next) {
            return panel(next).above();
        }
    }

    public final List<Village> villages = new ArrayList<>();
    public final Set<String> cuts = new HashSet<>();
    /** Fiber path blocks that were removed (packed positions); chunks never generated count as intact. */
    public final Set<Long> brokenFiber = new HashSet<>();
    public final Map<String, Integer> cableBlocks = new HashMap<>();
    public final Map<String, long[]> nicPositions = new HashMap<>();
    public final Map<UUID, Integer> alwaysOn = new HashMap<>();
    /** Generated fiber ends: "village:next|prev" -> {router port exit, panel} (packed positions). */
    public final Map<String, long[]> ends = new ConcurrentHashMap<>();
    /** Ends whose cabling has been seen loaded at least once. */
    public final Set<String> observed = ConcurrentHashMap.newKeySet();
    /** Patch panels and player spans placed against a ring path block (packed positions). */
    public final Set<Long> taps = ConcurrentHashMap.newKeySet();
    private RingPieces pieces;
    private static final ExecutorService BOOT =
            Executors.newFixedThreadPool(
                    2,
                    r -> {
                        Thread t = new Thread(r, "ECM-Infrastructure-Boot");
                        t.setDaemon(true);
                        return t;
                    });
    /** Planned sites per world seed, read by world generation (placement, structure). */
    public static final Map<Long, List<Village>> SITES = new ConcurrentHashMap<>();
    /** Fiber ring per world seed, read by the fiber feature during world generation. */
    public static final Map<Long, FiberChords> FIBER = new ConcurrentHashMap<>();
    private volatile boolean stopping;
    private FiberChords ring;
    private int chat;
    private final Map<String, Boolean> lastEnd = new ConcurrentHashMap<>();

    public static WorldNetwork get(ServerLevel level) {
        return level.getServer()
                .overworld()
                .getDataStorage()
                .computeIfAbsent(
                        new Factory<>(WorldNetwork::new, WorldNetwork::load), "ecm_network");
    }

    private static WorldNetwork load(CompoundTag tag, HolderLookup.Provider lookup) {
        WorldNetwork d = new WorldNetwork();
        ListTag list = tag.getList("villages", Tag.TAG_COMPOUND);
        for (int i = 0; i < list.size(); i++) {
            CompoundTag v = list.getCompound(i);
            // Saves from before the two-panel masts lack the panel sides: re-plan those.
            if (!v.contains("style") || !v.contains("nextSide")) {
                d.villages.clear();
                break;
            }
            d.villages.add(new Village(v.getInt("number"), v.getInt("x"), v.getInt("z"), v.getString("style"),
                    v.getInt("groundY"), v.getInt("endY"), v.getInt("prevSide"), v.getInt("nextSide")));
        }
        for (Tag c : tag.getList("cuts", Tag.TAG_STRING)) d.cuts.add(c.getAsString());
        for (long p : tag.getLongArray("brokenFiber")) d.brokenFiber.add(p);
        CompoundTag blocks = tag.getCompound("cableBlocks");
        for (String key : blocks.getAllKeys()) d.cableBlocks.put(key, blocks.getInt(key));
        CompoundTag nics = tag.getCompound("nicPositions");
        for (String key : nics.getAllKeys()) d.nicPositions.put(key, nics.getLongArray(key));
        CompoundTag nodes = tag.getCompound("alwaysOn");
        for (String key : nodes.getAllKeys())
            try {
                d.alwaysOn.put(UUID.fromString(key), nodes.getInt(key));
            } catch (IllegalArgumentException ignored) {
            }
        CompoundTag fiberEnds = tag.getCompound("fiberEnds");
        for (String key : fiberEnds.getAllKeys()) d.ends.put(key, fiberEnds.getLongArray(key));
        for (Tag t : tag.getList("observedEnds", Tag.TAG_STRING)) d.observed.add(t.getAsString());
        for (long p : tag.getLongArray("fiberTaps")) d.taps.add(p);
        return d;
    }

    public synchronized CompoundTag save(CompoundTag tag, HolderLookup.Provider lookup) {
        ListTag list = new ListTag();
        for (Village v : villages) {
            CompoundTag n = new CompoundTag();
            n.putInt("number", v.number);
            n.putInt("x", v.x);
            n.putInt("z", v.z);
            n.putString("style", v.style);
            n.putInt("groundY", v.groundY);
            n.putInt("endY", v.endY);
            n.putInt("prevSide", v.prevSide);
            n.putInt("nextSide", v.nextSide);
            list.add(n);
        }
        tag.put("villages", list);
        ListTag cut = new ListTag();
        for (String c : cuts) cut.add(StringTag.valueOf(c));
        tag.put("cuts", cut);
        tag.putLongArray("brokenFiber", brokenFiber.stream().mapToLong(Long::longValue).toArray());
        CompoundTag blocks = new CompoundTag();
        cableBlocks.forEach(blocks::putInt);
        tag.put("cableBlocks", blocks);
        CompoundTag nics = new CompoundTag();
        nicPositions.forEach(nics::putLongArray);
        tag.put("nicPositions", nics);
        CompoundTag nodes = new CompoundTag();
        alwaysOn.forEach((id, ports) -> nodes.putInt(id.toString(), ports));
        tag.put("alwaysOn", nodes);
        CompoundTag fiberEnds = new CompoundTag();
        ends.forEach(fiberEnds::putLongArray);
        tag.put("fiberEnds", fiberEnds);
        ListTag seen = new ListTag();
        for (String s : observed) seen.add(StringTag.valueOf(s));
        tag.put("observedEnds", seen);
        tag.putLongArray("fiberTaps", taps.stream().mapToLong(Long::longValue).toArray());
        return tag;
    }

    private static final List<TagKey<Biome>> STYLE_TAGS = List.of(BiomeTags.HAS_VILLAGE_PLAINS,
            BiomeTags.HAS_VILLAGE_DESERT, BiomeTags.HAS_VILLAGE_SAVANNA, BiomeTags.HAS_VILLAGE_SNOWY, BiomeTags.HAS_VILLAGE_TAIGA);

    /** The vanilla village style for a biome; plains for biomes without villages. */
    public static String style(Holder<Biome> biome) {
        for (int i = 0; i < STYLE_TAGS.size(); i++) if (biome.is(STYLE_TAGS.get(i))) return STYLES.get(i);
        return "plains";
    }

    public static ResourceLocation ispTemplate(String style) {
        return ResourceLocation.fromNamespaceAndPath("evanscomputermod", "tech_village/isp_" + style);
    }

    public static ResourceLocation datacenterTemplate(String style) {
        return ResourceLocation.fromNamespaceAndPath("evanscomputermod", "tech_village/datacenter_" + style);
    }

    /**
     * Height of the fiber endpoints above the start piece's ground: the mast top's template
     * y (the panels hang beside it) plus one, as the start piece's y = 0 layer sits at
     * groundY - 1.
     */
    public static int endpointOffset(ServerLevel level, String style) {
        var scan = com.example.evanscomputermod.worldgen.TemplateScan.of(level.getStructureManager(), ispTemplate(style))
                .orElseThrow(() -> new IllegalStateException("Missing ISP template " + ispTemplate(style)));
        int top = scan.mastTop();
        if (top < 0) throw new IllegalStateException("ISP template has no mast: " + style);
        return top;
    }

    /** The village whose data center runs the ring's chat server: random, fixed by the seed. */
    public static int chatVillage(long seed) {
        return 1 + (int) Math.floorMod(new SplittableRandom(seed ^ 0x4348415453455256L).nextLong(), 10L);
    }

    public int chatVillage() {
        return chat;
    }

    public synchronized void plan(ServerLevel level) {
        chat = chatVillage(level.getSeed());
        if (villages.isEmpty()) {
            var generator = level.getChunkSource().getGenerator();
            var random = level.getChunkSource().randomState();
            var sampler = random.sampler();
            BlockPos spawn = level.getSharedSpawnPos();
            double offset = RandomSource.create(level.getSeed()).nextDouble() * Math.PI * 2;
            int[][] site = new int[10][];
            String[] styles = new String[10];
            for (int i = 1; i <= 10; i++) {
                double angle = offset + (i - 1) * Math.PI / 5;
                int x = spawn.getX() + (int) Math.round(5000 * Math.cos(angle));
                int z = spawn.getZ() + (int) Math.round(5000 * Math.sin(angle));
                // Steer the site to the nearest biome that has vanilla villages.
                var found =
                        generator
                                .getBiomeSource()
                                .findBiomeHorizontal(
                                        x,
                                        64,
                                        z,
                                        1536,
                                        16,
                                        b -> STYLE_TAGS.stream().anyMatch(b::is),
                                        RandomSource.create(level.getSeed() + i),
                                        true,
                                        sampler);
                if (found != null) {
                    x = found.getFirst().getX();
                    z = found.getFirst().getZ();
                }
                x = (x >> 4) << 4;
                z = (z >> 4) << 4;
                // Exactly JigsawStructure's start height: start_height 0 + first free WORLD_SURFACE_WG
                // height at the start piece's bounding-box centre, which is the mast column.
                int ground = generator.getFirstFreeHeight(x, z, Heightmap.Types.WORLD_SURFACE_WG, level, random);
                var biome = generator.getBiomeSource().getNoiseBiome(
                        QuartPos.fromBlock(x), QuartPos.fromBlock(ground), QuartPos.fromBlock(z), sampler);
                styles[i - 1] = style(biome);
                site[i - 1] = new int[] {x, z, ground};
            }
            for (int i = 0; i < 10; i++) {
                int[] s = site[i], p = site[(i + 9) % 10], n = site[(i + 1) % 10];
                int[] sides = FiberChords.sides(s[0], s[1], p[0], p[1], n[0], n[1]);
                // FiberChords side numbers are Direction 3D data values (NORTH 2 ... EAST 5).
                villages.add(new Village(i + 1, s[0], s[1], styles[i], s[2], s[2] + endpointOffset(level, styles[i]),
                        sides[0], sides[1]));
            }
            setDirty();
            EvansComputerMod.LOGGER.info("Tech Village ring planned (chat server in village {}): {}", chat, villages);
        }
        SITES.put(level.getSeed(), List.copyOf(villages));
        if (ring == null) {
            ring = new FiberChords(
                    villages.stream().map(v -> pack(v.endpoint(true))).toList(),
                    villages.stream().map(v -> pack(v.endpoint(false))).toList());
            FIBER.put(level.getSeed(), ring);
        }
    }

    private static int[] pack(BlockPos p) {
        return new int[] {p.getX(), p.getY(), p.getZ()};
    }

    public FiberChords ring(ServerLevel level) {
        plan(level);
        return ring;
    }

    public static Village siteAt(long seed, int chunkX, int chunkZ) {
        for (Village v : SITES.getOrDefault(seed, List.of()))
            if ((v.x >> 4) == chunkX && (v.z >> 4) == chunkZ) return v;
        return null;
    }

    public UUID identity(ServerLevel level, int village, String role) {
        return UUID.nameUUIDFromBytes(
                (level.getSeed() + ":" + village + ":" + role).getBytes(StandardCharsets.UTF_8));
    }

    public Village nearest(BlockPos pos) {
        return villages.stream()
                .min(
                        Comparator.comparingDouble(
                                v -> Math.pow(v.x - pos.getX(), 2) + Math.pow(v.z - pos.getZ(), 2)))
                .orElseThrow();
    }

    public static String linkName(int a, int b) {
        return Math.min(a, b) + "-" + Math.max(a, b);
    }

    /** Chord index (0-based) joining village i and its next neighbour. */
    public static int chord(int a, int b) {
        int lo = Math.min(a, b), hi = Math.max(a, b);
        return lo == 1 && hi == 10 ? 9 : lo - 1;
    }

    /** {from, to} of the ring edge a-b, oriented so that to is from's next village. */
    private static int[] oriented(int a, int b) {
        return (a % 10) + 1 == b ? new int[] {a, b} : new int[] {b, a};
    }

    public void link(ServerLevel level, int a, int b, boolean intact) {
        String name = linkName(a, b);
        boolean changed = intact ? cuts.remove(name) : cuts.add(name);
        if (changed) {
            setDirty();
            ringChanged();
        }
        applyLinks(level);
    }

    /** No administrative cut and no missing path block on the chord between a and b. */
    public boolean fiberIntact(ServerLevel level, int a, int b) {
        return !cuts.contains(linkName(a, b)) && ring(level).intact(chord(a, b), brokenFiber);
    }

    /**
     * The fiber link between neighbours a and b is up: a's port toward b and b's port toward
     * a are on one segment (cable up each mast, panel, an unbroken run of fiber; or stand-ins
     * for a village that does not exist yet).
     */
    public boolean linkUp(ServerLevel level, int a, int b) {
        int[] o = oriented(a, b);
        CableNetworkManager mgr = CableNetworkManager.getInstance();
        return mgr != null
                && mgr.areOnSameNetwork(mac(level, o[0], ISP_ROUTER, FIBER_NEXT_PORT), mac(level, o[1], ISP_ROUTER, FIBER_PREV_PORT));
    }

    private static String endKey(int village, boolean next) {
        return village + ":" + (next ? "next" : "prev");
    }

    /** Generation records where a village's router fiber port and panel are (TechNetworkPiece). */
    public void recordEnd(int village, boolean next, BlockPos portExit, BlockPos panel) {
        long[] value = {portExit.asLong(), panel.asLong()};
        long[] old = ends.put(endKey(village, next), value);
        if (old == null || !Arrays.equals(old, value)) {
            setDirty();
            CableNetworkManager mgr = CableNetworkManager.getInstance();
            if (mgr != null) mgr.invalidateCache();
        }
    }

    /** The chord piece a village's fiber end sits on: index 0 of its "next" chord, the last of its "previous" one. */
    private int endPiece(ServerLevel level, int village, boolean next) {
        FiberChords r = ring(level);
        int c = next ? village - 1 : (village + 8) % 10;
        return pieces(level).piece(c, next ? 0 : r.path(c).length - 1);
    }

    /**
     * Is the router's fiber port on the same segment as its end of the fiber (cable up the
     * mast, panel)? Before the village has been seen loaded it stands in as plugged in.
     */
    public boolean endCabled(ServerLevel level, int village, boolean next) {
        if (!observed.contains(endKey(village, next))) return true;
        CableNetworkManager mgr = CableNetworkManager.getInstance();
        if (mgr == null) return false;
        var r = mgr.result();
        int piece = endPiece(level, village, next);
        Integer seg = mgr.networkOf(mac(level, village, ISP_ROUTER, next ? FIBER_NEXT_PORT : FIBER_PREV_PORT));
        return r != null && piece >= 0 && seg != null && r.pieceSegment[piece] == seg;
    }

    /** A human-readable end state for /ecm net links and /ecm techvillage info. */
    public String endState(ServerLevel level, int village, boolean next) {
        String key = endKey(village, next);
        long[] e = ends.get(key);
        if (e == null) return "not generated (stands in as cabled)";
        if (!observed.contains(key)) return "generated, never loaded (stands in as cabled)";
        boolean loaded = level.isLoaded(BlockPos.of(e[0])) && level.isLoaded(BlockPos.of(e[1]));
        if (endPiece(level, village, next) < 0) return "FIBER ENDPOINT MISSING";
        return (endCabled(level, village, next) ? "cabled" : "CABLE CUT") + (loaded ? "" : " (unloaded, last known)");
    }

    /** A fiber block left the world: on a chord path it breaks that chord there; next to one it was a tap. */
    public void fiberRemoved(ServerLevel level, BlockPos pos) {
        long p = FiberChords.pack(pos.getX(), pos.getY(), pos.getZ());
        if (ring(level).chordsAt(p) != 0) {
            if (brokenFiber.add(p)) {
                EvansComputerMod.LOGGER.info("Ring fiber removed at {} (chord mask {}): the chord is cut there until a span is put back",
                        pos.toShortString(), ring(level).chordsAt(p));
                setDirty();
                ringChanged();
            }
        } else attachmentChanged(level, pos, false);
    }

    /** A Fiber Span was placed: on its path position it repairs that break; next to the path it taps it. */
    public void fiberPlaced(ServerLevel level, BlockPos pos) {
        long p = FiberChords.pack(pos.getX(), pos.getY(), pos.getZ());
        if (brokenFiber.remove(p)) {
            setDirty();
            ringChanged();
        } else if (ring(level).chordsAt(p) == 0) attachmentChanged(level, pos, true);
    }

    /**
     * A patch panel or a player's Fiber Span appeared or disappeared at {@code pos}: if it
     * touches a ring path block it is (or was) a tap on that piece.
     */
    public void attachmentChanged(ServerLevel level, BlockPos pos, boolean placed) {
        if (level.dimension() != net.minecraft.world.level.Level.OVERWORLD) return;
        long p = pos.asLong();
        if (!RingPieces.touchesPath(ring(level), p)) return;
        if (placed ? taps.add(p) : taps.remove(p)) {
            EvansComputerMod.LOGGER.info("Ring tap {} at {}", placed ? "attached" : "removed", pos.toShortString());
            setDirty();
            ringChanged();
        }
    }

    private void ringChanged() {
        pieces = null;
        CableNetworkManager mgr = CableNetworkManager.getInstance();
        if (mgr != null) mgr.invalidateCache();
    }

    /** The ring split at its breaks and admin cuts, with its end panels and taps attached. */
    public synchronized RingPieces pieces(ServerLevel level) {
        if (pieces == null) {
            FiberChords r = ring(level);
            Map<Integer, int[]> admin = new HashMap<>();
            for (String c : cuts) {
                String[] ab = c.split("-");
                try {
                    int ch = chord(Integer.parseInt(ab[0]), Integer.parseInt(ab[1]));
                    admin.put(ch, new int[] {r.midpoint(ch)});
                } catch (RuntimeException ignored) {
                }
            }
            List<Long> attached = new ArrayList<>(taps);
            for (Village v : villages) {
                attached.add(v.panel(true).asLong());
                attached.add(v.panel(false).asLong());
            }
            pieces = new RingPieces(r, ringDim, brokenFiber, admin, attached);
        }
        return pieces;
    }

    private byte[] mac(ServerLevel level, int village, String role, int port) {
        return NetworkHub.deriveMac(identity(level, village, role), port);
    }

    private CableNetworkManager.MacAddress key(ServerLevel level, int village, String role, int port) {
        return new CableNetworkManager.MacAddress(mac(level, village, role, port));
    }

    // ===== CableNetworkManager.Topology =====

    private ServerLevel overworld;
    private int ringDim;

    @Override
    public SegmentGraph.Ring ring(int dim) {
        if (overworld == null) return null;
        if (dim != ringDim) {
            ringDim = dim;
            pieces = null;
        }
        return pieces(overworld);
    }

    /**
     * Stand-ins while a village's buildings have not been seen loaded: its fiber ports
     * count as plugged into their chord ends, and its data center servers as cabled to
     * the router's eth1. Once seen, the real (or last-known) cables decide.
     */
    @Override
    public void standIns(List<SegmentGraph.StandIn<CableNetworkManager.MacAddress>> out,
            List<SegmentGraph.Edge<CableNetworkManager.MacAddress>> edges) {
        ServerLevel level = overworld;
        if (level == null) return;
        for (Village v : villages) {
            int i = v.number();
            for (boolean next : new boolean[] {true, false}) {
                if (observed.contains(endKey(i, next))) continue;
                int piece = endPiece(level, i, next);
                if (piece >= 0) out.add(new SegmentGraph.StandIn<>(key(level, i, ISP_ROUTER, next ? FIBER_NEXT_PORT : FIBER_PREV_PORT), piece));
            }
            if (!observed.contains(i + ":dc")) {
                edges.add(new SegmentGraph.Edge<>(key(level, i, ISP_ROUTER, SERVER_PORT), key(level, i, WEB, SERVER_NIC), false));
                if (i == chat)
                    edges.add(new SegmentGraph.Edge<>(key(level, i, ISP_ROUTER, SERVER_PORT), key(level, i, CHAT, SERVER_NIC), false));
            }
        }
    }

    /** After a recompute: mark ends seen loaded, log end changes, drop stale taps. */
    @Override
    public void computed(SegmentGraph.Result<CableNetworkManager.MacAddress> result) {
        ServerLevel level = overworld;
        CableNetworkManager mgr = CableNetworkManager.getInstance();
        if (level == null || mgr == null) return;
        boolean changed = false;
        for (var e : ends.entrySet()) {
            BlockPos exit = BlockPos.of(e.getValue()[0]), panel = BlockPos.of(e.getValue()[1]);
            if (level.isLoaded(exit) && level.isLoaded(panel) && observed.add(e.getKey())) changed = true;
        }
        for (Village v : villages) {
            String dc = v.number() + ":dc";
            if (observed.contains(dc) || !ends.containsKey(endKey(v.number(), true))) continue;
            BlockPos a = mgr.exitOf(mac(level, v.number(), ISP_ROUTER, SERVER_PORT)), b = mgr.exitOf(mac(level, v.number(), WEB, SERVER_NIC));
            if (a != null && b != null && level.isLoaded(a) && level.isLoaded(b) && observed.add(dc)) changed = true;
        }
        for (var e : ends.keySet()) {
            if (!observed.contains(e)) continue;
            String[] k = e.split(":");
            int village = Integer.parseInt(k[0]);
            boolean next = k[1].equals("next");
            int piece = endPiece(level, village, next);
            Integer seg = result.segment.get(key(level, village, ISP_ROUTER, next ? FIBER_NEXT_PORT : FIBER_PREV_PORT));
            boolean ok = piece >= 0 && seg != null && result.pieceSegment[piece] == seg;
            Boolean before = lastEnd.put(e, ok);
            if (before != null && before != ok)
                EvansComputerMod.LOGGER.info("Tech Village {} fiber end toward the {} village: {}", village, next ? "next" : "previous",
                        ok ? "cabled again" : "CABLE CUT");
        }
        for (Long t : List.copyOf(taps)) {
            BlockPos p = BlockPos.of(t);
            if (level.isLoaded(p) && !(level.getBlockState(p).getBlock() instanceof com.example.evanscomputermod.block.FiberPatchPanelBlock
                    || level.getBlockState(p).getBlock() instanceof com.example.evanscomputermod.block.FiberInfrastructureBlock)) {
                taps.remove(t);
                pieces = null;
                changed = true;
            }
        }
        if (changed) {
            setDirty();
            mgr.invalidateCache();
        }
    }

    /**
     * Describe a chord for /ecm net links: whether the two routers share a segment, its
     * pieces, breaks and taps, and both ends.
     */
    public List<String> describeChord(ServerLevel level, int a) {
        int b = a == 10 ? 1 : a + 1, c = chord(a, b);
        String name = linkName(a, b);
        RingPieces p = pieces(level);
        FiberChords r = ring(level);
        CableNetworkManager mgr = CableNetworkManager.getInstance();
        boolean up = linkUp(level, a, b);
        boolean carrier = mgr != null && mgr.carrierOf(mac(level, a, ISP_ROUTER, FIBER_NEXT_PORT))
                && mgr.carrierOf(mac(level, b, ISP_ROUTER, FIBER_PREV_PORT));
        List<String> out = new ArrayList<>();
        int[] pieceIds = p.piecesOf(c);
        out.add(name + ": " + (up ? "connected" : "CUT") + (carrier ? ", carrier up" : ", no carrier") + "; "
                + r.path(c).length + " blocks in " + pieceIds.length + " piece(s); " + a + " eth3 " + endState(level, a, true)
                + ", " + b + " eth2 " + endState(level, b, false));
        StringBuilder breaks = new StringBuilder();
        for (int i : p.breaks(c)) {
            int[] at = r.path(c)[i];
            boolean admin = cuts.contains(name) && i == r.midpoint(c)
                    && !brokenFiber.contains(FiberChords.pack(at[0], at[1], at[2]));
            breaks.append(breaks.length() == 0 ? "" : "; ").append(admin ? "admin cut" : "span missing").append(" at ")
                    .append(at[0]).append(' ').append(at[1]).append(' ').append(at[2]).append(" (block ").append(i).append(')');
        }
        if (breaks.length() > 0) out.add("  breaks: " + breaks);
        var result = mgr == null ? null : mgr.result();
        for (var e : p.attachedTo(c).entrySet())
            for (long t : e.getValue()) {
                if (!taps.contains(t)) continue;
                BlockPos at = BlockPos.of(t);
                int piece = p.piece(c, e.getKey());
                int seg = result == null || piece < 0 ? -1 : result.pieceSegment[piece];
                int nics = seg < 0 ? 0 : result.members.getOrDefault(seg, List.of()).size();
                out.add("  tap at " + at.getX() + " " + at.getY() + " " + at.getZ() + " (block " + e.getKey() + ", piece "
                        + (piece < 0 ? "-" : piece - pieceIds[0] + 1) + "; " + nics + " NIC(s) on that segment)");
            }
        return out;
    }

    public void applyLinks(ServerLevel level) {
        CableNetworkManager mgr = CableNetworkManager.getInstance();
        if (mgr == null) return;
        plan(level);
        overworld = level.getServer().overworld();
        mgr.setTopology(this);
        // The real internet: village 1's uplink reaches the host gateway (logical).
        mgr.logicalLink("internet-isp1", mac(level, 1, ISP_ROUTER, UPLINK_PORT), InternetProxy.MAC, true);
        mgr.flush();
    }
    /** The infrastructure computers of village {@code n}: ISP router, web server, chat server. */
    public List<String> infrastructure(int n) {
        return n == chat ? List.of(ISP_ROUTER, WEB, CHAT) : List.of(ISP_ROUTER, WEB);
    }

    public static void start(ServerLevel level) {
        WorldNetwork d = get(level);
        d.plan(level);
        d.applyLinks(level);
        if (System.getProperty("neoforge.enabledGameTestNamespaces") != null) return;
        if (Boolean.parseBoolean(System.getProperty("evanscomputermod.techVillages", "true")))
            for (Village v : d.villages)
                for (String role : d.infrastructure(v.number))
                    d.boot(level, v.number, role);
        int count = 0;
        for (var node : Map.copyOf(d.alwaysOn).entrySet())
            if (count++ < Math.max(0, Integer.getInteger("evanscomputermod.headlessCap", 64)))
                d.bootInstance(level, node.getKey(), false, node.getValue());
    }

    public void boot(ServerLevel level, int number, String role) {
        UUID id = identity(level, number, role);
        ComputerHost host = ComputerHost.get(level.getServer(), id);
        host.setInfrastructure(true);
        if (host.instance() != null) return;
        provision(level, number, role);
        bootInstance(level, id, true, role.equals(ISP_ROUTER) ? 9 : 5);
    }

    private void bootInstance(ServerLevel level, UUID id, boolean infrastructure, int ports) {
        ComputerHost host = ComputerHost.get(level.getServer(), id);
        host.setInfrastructure(infrastructure);
        if (!host.beginBoot()) return;
        BOOT.execute(
                () -> {
                    ComputerInstance instance = null;
                    try {
                        int safePorts = Math.max(1, Math.min(32, ports));
                        byte[][] macs = new byte[safePorts][];
                        for (int i = 0; i < safePorts; i++) macs[i] = NetworkHub.deriveMac(id, i);
                        instance = new ComputerInstance(host, macs);
                        instance.loadModule("terminal_os.wasm");
                        ComputerInstance ready = instance;
                        if (stopping) {
                            ready.close();
                            host.endBoot();
                            return;
                        }
                        level.getServer()
                                .execute(
                                        () -> {
                                            host.endBoot();
                                            if (stopping) {
                                                ready.close();
                                                return;
                                            }
                                            if (host.instance() != null) {
                                                ready.close();
                                                return;
                                            }
                                            host.track(ready);
                                            if (host.attachment()
                                                            instanceof
                                                            com.example.evanscomputermod.block
                                                                                    .TerminalBlockEntity
                                                                            block
                                                    && !block.isRemoved())
                                                block.attachLiveComputer(ready);
                                            try {
                                                ready.executeMain();
                                                ready.startWorkerThread();
                                            } catch (Exception e) {
                                                ready.close();
                                                host.forget();
                                                EvansComputerMod.LOGGER.error(
                                                        "Infrastructure boot failed", e);
                                            }
                                        });
                    } catch (Exception e) {
                        host.endBoot();
                        if (instance != null) instance.close();
                        EvansComputerMod.LOGGER.error("Infrastructure load failed", e);
                    }
                });
    }

    public void stop() {
        stopping = true;
    }

    /**
     * Version of the generated infrastructure configuration (ISP router, data center
     * servers). 2: /28 ring links with open peering for taps.
     */
    public static final int PROVISION_VERSION = 2;
    /** Marker file in an infrastructure computer's directory: "version N". */
    public static final String PROVISION_MARKER = ".ecm-provision";

    public static boolean infrastructureRole(String role) {
        return role.equals(ISP_ROUTER) || role.equals(WEB) || role.equals(CHAT);
    }

    /**
     * Write a computer's startup files. Missing files are always written; existing ones are
     * kept, except on infrastructure computers provisioned by an older version of the mod
     * (no or an older {@link #PROVISION_MARKER}), whose files are rewritten once. Player
     * edits made after that (e.g. {@code write memory} on the ISP router) are kept, and
     * house computers are never rewritten.
     */
    public void provision(ServerLevel level, int number, String role) {
        plan(level);
        Path root = ComputerStorage.path(level.getServer(), identity(level, number, role));
        try {
            Files.createDirectories(root);
            Path marker = root.resolve(PROVISION_MARKER);
            boolean upgrade = infrastructureRole(role) && provisionedVersion(marker) < PROVISION_VERSION;
            for (var e : configs(number, role).entrySet()) {
                Path p = root.resolve(e.getKey());
                Files.createDirectories(p.getParent());
                if (upgrade || !Files.exists(p)) Files.writeString(p, e.getValue());
            }
            if (upgrade) {
                Files.writeString(marker, "version " + PROVISION_VERSION + "\n");
                EvansComputerMod.LOGGER.info("Provisioned village {} {} (configuration version {})", number, role, PROVISION_VERSION);
            }
        } catch (Exception e) {
            throw new IllegalStateException("Cannot provision " + number + "/" + role, e);
        }
    }

    /** The provisioning version recorded in a computer directory's marker (0: none). */
    public static int provisionedVersion(Path marker) {
        try {
            if (!Files.exists(marker)) return 0;
            String[] v = Files.readString(marker).trim().split("\\s+");
            return v.length == 2 && v[0].equals("version") ? Integer.parseInt(v[1]) : 0;
        } catch (Exception e) {
            return 0;
        }
    }

    /** Startup files for this world (chat server village and styles from the plan). */
    public Map<String, String> configs(int i, String role) {
        List<String> styles = villages.stream().map(Village::style).toList();
        return configs(i, role, chat, styles);
    }

    /** Address of the ring's chat server when it lives in village {@code chat}. */
    public static String chatAddress(int chat) {
        return "100." + (64 + chat) + ".0.20";
    }

    /** The /etc/chat.conf every village computer gets. */
    public static String chatConf(int chat, String nick) {
        return "# Tech Village chat: run 'chat' (see 'chat --help')\nserver " + chatAddress(chat) + "\nport "
                + CHAT_PORT + "\nnick " + nick + "\n";
    }

    /**
     * Startup files. ISP router: eth0 (DOWN) village access LAN 100.(64+N).1.1/24 with a
     * DHCP pool for every house router and player PC on the village cable; eth1 (UP)
     * server LAN 100.(64+N).0.1/24 to the data center, with a small pool for computers
     * players add to its racks; eth2/eth3 fiber to the previous/next village; eth4
     * village 1's uplink to the host gateway; eth5-eth8 spare (player AS peering).
     * Data center servers use their UP face (eth1): web 100.(64+N).0.10, chat
     * 100.(64+N).0.20. House router: eth0 (DOWN) WAN by DHCP with NAT, eth1 (UP) private
     * LAN; house PC eth1 by DHCP. DOWN/UP are eth0/eth1 for any horizontal facing.
     *
     * @param chat the village whose data center runs the chat server
     * @param styles the ten villages' styles (for the website), may be empty
     */
    public static Map<String, String> configs(int i, String role, int chat, List<String> styles) {
        int previous = i == 1 ? 10 : i - 1, next = i == 10 ? 1 : i + 1, oct = 64 + i;
        Map<String, String> files = new HashMap<>();
        if (role.equals(ISP_ROUTER)) {
            files.put("router.cfg", ispRouterConfig(i));
            files.put("services.cfg", "router on\nsshd &\n");
            files.put("etc/chat.conf", chatConf(chat, "v" + i + "-isp"));
        } else if (role.equals(WEB)) {
            files.put(
                    "network.cfg",
                    "iface eth1 100." + oct + ".0.10/24\nroute default via 100." + oct + ".0.1 dev eth1\n");
            files.put("services.cfg", "httpd 80 &\nsshd &\n");
            files.put("index.html", website(i, chat, styles));
            files.put("etc/chat.conf", chatConf(chat, "v" + i + "-web"));
        } else if (role.equals(CHAT)) {
            files.put(
                    "network.cfg",
                    "iface eth1 100." + oct + ".0.20/24\nroute default via 100." + oct + ".0.1 dev eth1\n");
            files.put("services.cfg", "chatd " + CHAT_PORT + " &\nsshd &\n");
            files.put("etc/chat.conf", chatConf(chat, "v" + i + "-chatd"));
        } else if (role.endsWith(".router")) {
            files.put("services.cfg", "router on\n");
            files.put(
                    "router.cfg",
                    "configure terminal\n"
                            + "ip routing\n"
                            + "interface eth0\n"
                            + "ip dhcp\n"
                            + "ip nat outside\n"
                            + "exit\n"
                            + "interface eth1\n"
                            + "ip address 192.168.1.1/24\n"
                            + "ip nat inside\n"
                            + "exit\n"
                            + "dhcp-server vrf default\n"
                            + "pool lan\n"
                            + "range 192.168.1.10 192.168.1.200\n"
                            + "default-router 192.168.1.1\n"
                            + "dns-server 1.1.1.1\n"
                            + "lease 86400\n"
                            + "enable\n"
                            + "end\n");
            files.put("etc/chat.conf", chatConf(chat, "v" + i + "-" + role.replace(".router", "") + "-rt"));
        } else {
            files.put("network.cfg", "iface eth1 dhcp\n");
            files.put("etc/chat.conf", chatConf(chat, "v" + i + "-" + role.replace(".pc", "")));
        }
        return files;
    }

    /**
     * Village {@code i}'s ISP router configuration. eth2/eth3 are the ring links:
     * 172.31.P.0/28 to the previous village (this router .2) and 172.31.i.0/28 to the next
     * (this router .1); .3-.14 are free for taps. Peer group TAPS accepts any eBGP speaker
     * that connects from either /28 (open peering), at most 8 per link and 20 prefixes
     * each, keeping only the tap's own prefixes up to /24: never the village ranges, the
     * ring links or a default route. Keep in step with scripts/gen-ring-scenarios.py.
     */
    public static String ispRouterConfig(int i) {
        int previous = i == 1 ? 10 : i - 1, next = i == 10 ? 1 : i + 1, oct = 64 + i;
        String cfg =
                "configure terminal\nip routing\n"
                        + "interface eth0\nip address 100." + oct + ".1.1/24\nexit\n"
                        + "interface eth1\nip address 100." + oct + ".0.1/24\nexit\n"
                        + "interface eth2\nip address 172.31." + previous + ".2/28\nexit\n"
                        + "interface eth3\nip address 172.31." + i + ".1/28\nexit\n";
        if (i == 1)
            cfg +=
                    "interface eth0\nip nat inside\nexit\n"
                            + "interface eth1\nip nat inside\nexit\n"
                            + "interface eth2\nip nat inside\nexit\n"
                            + "interface eth3\nip nat inside\nexit\n"
                            + "interface eth4\nip address 10.0.0.2/24\nip nat outside\nexit\n"
                            + "ip route 0.0.0.0/0 10.0.0.1 eth4\n";
        cfg +=
                "dhcp-server vrf default\npool village\nrange 100."
                        + oct + ".1.10 100." + oct + ".1.200\ndefault-router 100." + oct + ".1.1\n"
                        + "dns-server 1.1.1.1\nlease 86400\nenable\nexit\n"
                        + "pool datacenter\nrange 100." + oct + ".0.100 100." + oct + ".0.199\ndefault-router 100."
                        + oct + ".0.1\ndns-server 1.1.1.1\nlease 86400\nenable\nexit\nexit\n";
        cfg +=
                "ip prefix-list TAP-IN seq 10 deny 100.64.0.0/10 le 32\n"
                        + "ip prefix-list TAP-IN seq 20 deny 172.31.0.0/16 le 32\n"
                        + "ip prefix-list TAP-IN seq 30 deny 0.0.0.0/0\n"
                        + "ip prefix-list TAP-IN seq 40 permit 0.0.0.0/0 le 24\n"
                        + "route-map TAP-IN permit 10\nmatch ip address prefix-list TAP-IN\nexit\n";
        cfg +=
                "router bgp " + (65000 + i)
                        + "\nbgp router-id 100." + oct + ".0.1\ntimers bgp 10 30\n"
                        + "neighbor 172.31." + previous + ".1 remote-as " + (65000 + previous) + "\n"
                        + "neighbor 172.31." + i + ".2 remote-as " + (65000 + next) + "\n"
                        + "neighbor TAPS peer-group\n"
                        + "neighbor TAPS remote-as external\n"
                        + "neighbor TAPS listen ip-range 172.31." + previous + ".0/28 limit 8\n"
                        + "neighbor TAPS listen ip-range 172.31." + i + ".0/28 limit 8\n"
                        + "address-family ipv4 unicast\n"
                        + "neighbor 172.31." + previous + ".1 activate\n"
                        + "neighbor 172.31." + i + ".2 activate\n"
                        + "neighbor TAPS activate\n"
                        + "neighbor TAPS route-map TAP-IN in\n"
                        + "neighbor TAPS maximum-prefix 20\n"
                        + "network 100." + oct + ".0.0/24\n"
                        + "network 100." + oct + ".1.0/24\n";
        if (i == 1) cfg += "network 0.0.0.0/0\n";
        return cfg + "end\n";
    }

    /** The village website served by its data center (index.html, also at "/"). */
    public static String website(int i, int chat, List<String> styles) {
        int previous = i == 1 ? 10 : i - 1, next = i == 10 ? 1 : i + 1, oct = 64 + i;
        String style = i <= styles.size() ? styles.get(i - 1) : "plains";
        StringBuilder html = new StringBuilder();
        html.append("<!DOCTYPE html>\n<html><head><title>Tech Village ").append(i).append(" - AS ")
                .append(65000 + i).append("</title></head>\n<body>\n");
        html.append("<h1>Tech Village ").append(i).append("</h1>\n");
        html.append("<p>A ").append(style).append(" village on the Tech Village fiber ring. AS ").append(65000 + i)
                .append(", served from the village data center.</p>\n");
        html.append("<h2>Network</h2>\n<ul>\n");
        html.append("<li>Village cable (ISP eth0): 100.").append(oct).append(".1.0/24, gateway 100.").append(oct)
                .append(".1.1, DHCP .10-.200: plug any computer in and use 'iface ethN dhcp'</li>\n");
        html.append("<li>Data center LAN (ISP eth1): 100.").append(oct).append(".0.0/24, gateway 100.").append(oct)
                .append(".0.1; this web server 100.").append(oct).append(".0.10; free racks get DHCP .100-.199</li>\n");
        html.append("<li>Fiber: eth2 172.31.").append(previous).append(".2/28 to village ").append(previous)
                .append(" (AS ").append(65000 + previous).append("), eth3 172.31.").append(i).append(".1/28 to village ")
                .append(next).append(" (AS ").append(65000 + next).append(")</li>\n");
        html.append("<li>Open peering on both fiber links: tap a span with a patch panel, take a free address .3-.14 in")
                .append(" that link's /28 and peer from any AS (eBGP; up to 20 of your own prefixes, /24 or shorter)</li>\n");
        if (i == 1) html.append("<li>Uplink: eth4 10.0.0.2/24 to the internet gateway 10.0.0.1 (NAT)</li>\n");
        html.append("<li>Spare ports for peering: eth5-eth8</li>\n</ul>\n");
        html.append("<h2>Chat</h2>\n<p>The ring's chat server runs in the data center of village ").append(chat)
                .append(": ").append(chatAddress(chat)).append(" port ").append(CHAT_PORT).append(".</p>\n")
                .append("<p>Run <code>chat</code> (village computers read /etc/chat.conf) or <code>chat ")
                .append(chatAddress(chat)).append("</code>. Type to talk; /nick NAME, /who, /quit.</p>\n");
        html.append("<h2>Other villages</h2>\n<ul>\n");
        for (int v = 1; v <= 10; v++) {
            if (v == i) continue;
            html.append("<li><a href=\"http://100.").append(64 + v).append(".0.10/\">Tech Village ").append(v)
                    .append("</a> (").append(v <= styles.size() ? styles.get(v - 1) : "?").append(", AS ")
                    .append(65000 + v).append(")</li>\n");
        }
        html.append("</ul>\n</body></html>\n");
        return html.toString();
    }
}
//?}
