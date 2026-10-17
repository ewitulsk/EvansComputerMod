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
    /** ISP router NICs, relative to its screen: bottom eth0, top eth1, back eth2, left eth3 (right: its Interface Block). */
    public static final int ACCESS_PORT = 0, SERVER_PORT = 1, FIBER_PREV_PORT = 2, FIBER_NEXT_PORT = 3, UPLINK_PORT = 4;
    /** Data center servers use their UP face (eth1) on the server LAN. */
    public static final int SERVER_NIC = 1;
    public static final String ISP_ROUTER = "isp.router", WEB = "datacenter.web", CHAT = "datacenter.chat";
    /** The carrier core (cored) in the core village's data center (Cellular v2 phase 5, {@link CellPlan}). */
    public static final String CORE = "datacenter.core";
    /** The package server ({@code pkg serve}) beside the chat server: the Tech Village repository ({@link VillageRepo}). */
    public static final String REPO = "datacenter.repo";
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
    /** The carrier network per world seed, read by world generation (span sites, Phone Stores). */
    public static final Map<Long, CellPlan> CELLS = new ConcurrentHashMap<>();
    /** The DNS plan per world seed, read by world generation (the data center's DNS racks). */
    public static final Map<Long, DnsPlan> DNS = new ConcurrentHashMap<>();
    private volatile boolean stopping;
    private FiberChords ring;
    private CellPlan cells;
    private DnsPlan dns;
    private int chat;
    private VillageRepo repo;
    /** The ring's accounts (authd on the chat server), from the seed. */
    private VillageAuth auth;
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

    /** This world's package repository (its server in the chat village, its key from the seed). */
    public VillageRepo repo(ServerLevel level) {
        plan(level);
        return repo;
    }

    public synchronized void plan(ServerLevel level) {
        chat = chatVillage(level.getSeed());
        if (repo == null) repo = VillageRepo.of(level.getSeed(), chat);
        if (auth == null) auth = VillageAuth.of(level.getSeed());
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
        if (cells == null) {
            cells = CellPlan.of(level.getSeed(), ring);
            CELLS.put(level.getSeed(), cells);
        }
        boolean onIsp = DnsConfig.resolverOnIspRouter();
        if (dns == null || dns.resolverOnIsp != onIsp) {
            dns = new DnsPlan(level.getSeed(), onIsp, chat, cells.coreVillage);
            DNS.put(level.getSeed(), dns);
            cells.dns = dns.resolver(cells.coreVillage);
        }
    }

    /** The ring's DNS: who runs which part, the files they start from. */
    public DnsPlan dns(ServerLevel level) {
        plan(level);
        return dns;
    }

    /** The DNS plan for a seed (world generation), or null before the plan. */
    public static DnsPlan dnsFor(long seed) {
        return DNS.get(seed);
    }

    /** The carrier network of this world (carrier, core, sites, SIMs). */
    public CellPlan cells(ServerLevel level) {
        plan(level);
        return cells;
    }

    /** The carrier network planned for a seed (world generation), or null before the plan. */
    public static CellPlan cellsFor(long seed) {
        return CELLS.get(seed);
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
            // The cell sites' taps along the chords (generated with the fiber, see CellSites).
            if (cells != null)
                for (CellPlan.SpanSite s : cells.allSpans()) attached.add(BlockPos.asLong(s.x(), s.panelY(), s.z()));
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
                if (i == chat) {
                    edges.add(new SegmentGraph.Edge<>(key(level, i, ISP_ROUTER, SERVER_PORT), key(level, i, CHAT, SERVER_NIC), false));
                    edges.add(new SegmentGraph.Edge<>(key(level, i, ISP_ROUTER, SERVER_PORT), key(level, i, REPO, SERVER_NIC), false));
                }
                if (cells != null && i == cells.coreVillage)
                    edges.add(new SegmentGraph.Edge<>(key(level, i, ISP_ROUTER, SERVER_PORT), key(level, i, CORE, SERVER_NIC), false));
                // The DNS racks (docs/DNS.md): the resolver and the village's part of the hierarchy.
                if (dns != null)
                    for (String role : dns.roles(i))
                        edges.add(new SegmentGraph.Edge<>(key(level, i, ISP_ROUTER, SERVER_PORT), key(level, i, role, SERVER_NIC), false));
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
        mgr.logicalLink("internet-isp1", mac(level, 1, ISP_ROUTER, UPLINK_PORT), com.example.evanscomputermod.api.internet.InternetUplinks.GATEWAY_MAC, true);
        mgr.flush();
    }
    /**
     * The infrastructure computers of village {@code n}: ISP router, web server, chat and package
     * servers, carrier core, and its part of the DNS hierarchy ({@link DnsPlan#roles}).
     */
    public List<String> infrastructure(int n) {
        List<String> out = new ArrayList<>(List.of(ISP_ROUTER, WEB));
        if (n == chat) {
            out.add(CHAT);
            out.add(REPO);
        }
        if (cells != null && n == cells.coreVillage) out.add(CORE);
        if (dns != null) out.addAll(dns.roles(n));
        return out;
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
     * servers). 2: /28 ring links with open peering for taps. 3: the carrier network (the
     * ring links are /26 for the cell sites along them and announced, the core's pool is
     * routed to the core village's data center, the website's Cellular section). 4: the Tech
     * Village package repository (its server, /etc/pkg/repos with the pinned key, the website's
     * Apps section). 5: service discovery (docs/DISCOVERY.md): the ISP router and the data
     * center servers run {@code disco serve}, the ISP answers for the ring's chat server and
     * repository, the core relays its phones' searches to it. 6: DNS (docs/DNS.md): the
     * resolvers, root servers, registries, registrar and IANA computers, {@code named} on every
     * web server with its village zone, DHCP and the carrier core handing out the village
     * resolver and the {@code vN.ring} search domain. 7: accounts and music (docs/AUTH.md,
     * docs/MUSIC.md): the chat server runs {@code authd} with the seed's realm key and
     * {@code discordd}/{@code musicd} trusting it, with the village songs in {@code /srv/music};
     * the ISP answers for {@code auth} and {@code music}; every computer has the realm pinned
     * for Discord and Music. The marker also records where the
     * resolvers run ({@link DnsConfig}); a change of that re-provisions once as well.
     */
    public static final int PROVISION_VERSION = 8;
    /** Files the program owning them rewrites (cored's subscriber database): written once, never upgraded. */
    private static final Set<String> WRITE_ONCE = Set.of("etc/cored.db");
    /** Marker file in an infrastructure computer's directory: "version N". */
    public static final String PROVISION_MARKER = ".ecm-provision";

    public static boolean infrastructureRole(String role) {
        return role.equals(ISP_ROUTER) || role.equals(WEB) || role.equals(CHAT) || role.equals(CORE) || role.equals(REPO)
                || DnsPlan.isDnsRole(role);
    }

    /** The marker text: the version and where the resolvers run. */
    private String markerText() {
        return "version " + PROVISION_VERSION + "\ndns " + (dns != null && dns.resolverOnIsp ? "isp" : "dedicated") + "\n";
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
            boolean upgrade = infrastructureRole(role)
                    && (provisionedVersion(marker) < PROVISION_VERSION || !Files.readString(marker).equals(markerText()));
            for (var e : configs(number, role).entrySet()) {
                Path p = root.resolve(e.getKey());
                Files.createDirectories(p.getParent());
                if ((upgrade && !WRITE_ONCE.contains(e.getKey())) || !Files.exists(p)) Files.writeString(p, e.getValue());
            }
            // The package server's repository is the mod's (and the music server's starting songs):
            // refreshed whenever the mod's build differs.
            Map<String, byte[]> binary = role.equals(REPO) ? repo.serverFiles() : role.equals(CHAT) ? VillageAuth.songs() : Map.of();
            if (!binary.isEmpty())
                for (var e : binary.entrySet()) {
                    Path p = root.resolve(e.getKey());
                    Files.createDirectories(p.getParent());
                    if (!Files.exists(p) || !Arrays.equals(Files.readAllBytes(p), e.getValue())) Files.write(p, e.getValue());
                }
            // cored owns its database; only the DNS server it hands its phones follows the plan.
            Path db = root.resolve("etc/cored.db");
            if (upgrade && role.equals(CORE) && cells != null && Files.exists(db)) {
                String text = Files.readString(db);
                String patched = text.replaceAll("(?m)^dns .*$", "dns " + cells.dns);
                if (!patched.equals(text)) Files.writeString(db, patched);
            }
            if (upgrade) {
                Files.writeString(marker, markerText());
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
        return configs(i, role, chat, styles, cells, repo, dns, auth);
    }

    /**
     * A display phone of village {@code village}'s Phone Store (slot 1-4): its computer, fixed by
     * the seed, with the Tech Village repository already in its {@code /etc/pkg/repos} (key
     * pinned). Nothing else is installed: the player finds Discord in the App Store.
     */
    public UUID provisionStorePhone(ServerLevel level, int village, int slot) {
        plan(level);
        UUID id = identity(level, village, "store.phone" + slot);
        Path root = ComputerStorage.path(level.getServer(), id);
        try {
            Path repos = root.resolve(VillageRepo.REPOS_FILE);
            Files.createDirectories(repos.getParent());
            if (!Files.exists(repos)) Files.writeString(repos, repo.reposFile());
            for (var e : auth.pinFiles(chat).entrySet()) {
                Path p = root.resolve(e.getKey());
                Files.createDirectories(p.getParent());
                if (!Files.exists(p)) Files.writeString(p, e.getValue());
            }
        } catch (Exception e) {
            throw new IllegalStateException("Cannot provision store phone " + village + "/" + slot, e);
        }
        return id;
    }

    /** The chat server's services.cfg line: the ring's Discord backend, its home server named {@link VillageDiscovery#DISCORD_NAME}. */
    public static String discordCommand() {
        return "discordd " + CHAT_PORT + " --name \"" + VillageDiscovery.DISCORD_NAME + "\" &";
    }

    /** Address of the ring's Discord backend (the chat server) when it lives in village {@code chat}. */
    public static String chatAddress(int chat) {
        return "100." + (64 + chat) + ".0.20";
    }

    /**
     * The /etc/browser.conf of village {@code i}'s house PCs: the Browser's default home page is
     * the village's own website, by name where the ring has DNS (docs/gui/BROWSER.md). A home
     * page the player sets in the Browser wins over it.
     */
    public static String browserConf(int i, DnsPlan dns) {
        String home = dns == null ? "http://100." + (64 + i) + ".0.10/" : "http://web.v" + i + ".ring/";
        return "# The Browser's home page (the Home button, and where it opens): this village's website.\n"
                + "# Menu > Set as home page in the Browser overrides it for that computer's player.\nhome " + home + "\n";
    }

    /** The /etc/chat.conf every village computer gets: the ring's Discord backend (Discord and the chat command read it). */
    public static String chatConf(int chat, String nick) {
        return "# Tech Village Discord backend: open Discord, or run 'chat' (see 'chat --help')\nserver " + chatAddress(chat) + "\nport "
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
        return configs(i, role, chat, styles, null);
    }

    /**
     * {@link #configs(int, String, int, List)} with the carrier network ({@code cells}, or null
     * for a ring without one): the core, the Phone Store's cell site and the span sites, and the
     * ISP's route to the core's phone pool.
     */
    public static Map<String, String> configs(int i, String role, int chat, List<String> styles, CellPlan cells) {
        return configs(i, role, chat, styles, cells, null);
    }

    /**
     * {@link #configs(int, String, int, List, CellPlan)} with the world's package repository
     * ({@code repo}, or null for none): every computer gets {@code /etc/pkg/repos} with it (key
     * pinned), the chat village's package server serves it, the website names it.
     */
    public static Map<String, String> configs(int i, String role, int chat, List<String> styles, CellPlan cells, VillageRepo repo) {
        return configs(i, role, chat, styles, cells, repo, null);
    }

    /**
     * {@link #configs(int, String, int, List, CellPlan, VillageRepo)} with the ring's DNS
     * ({@code dns}, or null for none, as in the router labs): every data center server gets
     * the village resolver and search domain, the web server runs {@code named} for its
     * village zone, the ISP's pools and the house routers hand the resolver out, and the
     * hierarchy's own computers get their zones and daemons.
     */
    public static Map<String, String> configs(int i, String role, int chat, List<String> styles, CellPlan cells, VillageRepo repo,
                                              DnsPlan dns) {
        return configs(i, role, chat, styles, cells, repo, dns, null);
    }

    /**
     * {@link #configs(int, String, int, List, CellPlan, VillageRepo, DnsPlan)} with the ring's
     * accounts ({@code auth}, or null for none): the chat server runs authd, discordd and musicd
     * on it, every computer gets the realm pinned for Discord and Music, and the ISP router's
     * discovery answers for auth and music.
     */
    public static Map<String, String> configs(int i, String role, int chat, List<String> styles, CellPlan cells, VillageRepo repo,
                                              DnsPlan dns, VillageAuth auth) {
        Map<String, String> files = configsWithoutAccounts(i, role, chat, styles, cells, repo, dns, auth);
        if (auth != null) files.putAll(auth.pinFiles(chat));
        return files;
    }

    private static Map<String, String> configsWithoutAccounts(int i, String role, int chat, List<String> styles, CellPlan cells,
                                                              VillageRepo repo, DnsPlan dns, VillageAuth auth) {
        Map<String, String> files = new HashMap<>();
        if (dns != null && DnsPlan.isDnsRole(role)) {
            if (repo != null) files.put(VillageRepo.REPOS_FILE, repo.reposFile());
            files.put("network.cfg", dns.serverNetwork(i, DnsPlan.host(role)));
            files.put("etc/resolv.conf", dns.resolvConf(i));
            files.put("services.cfg", dns.services(i, role));
            files.putAll(dns.files(i, role));
            files.put("etc/chat.conf", chatConf(chat, "v" + i + "-" + role.replace("datacenter.", "")));
            return files;
        }
        if (repo != null) files.put(VillageRepo.REPOS_FILE, repo.reposFile());
        if (cells != null && (role.equals(CORE) || role.equals(CellPlan.STORE_ROLE) || CellPlan.spanK(role) > 0)) {
            cellConfigs(i, role, chat, cells, files);
            if (dns != null && files.containsKey("network.cfg"))
                files.put("network.cfg", files.get("network.cfg") + "dns " + dns.resolver(i) + "\n");
            if (dns != null) files.put("etc/resolv.conf", dns.resolvConf(i));
            return files;
        }
        int oct = 64 + i;
        // Data center servers ask their village's resolver, with vN.ring as the search domain.
        String dnsLine = dns == null ? "" : "dns " + dns.resolver(i) + "\n";
        boolean house = role.endsWith(".router") && !role.equals(ISP_ROUTER) || role.endsWith(".pc");
        if (dns != null && !house) files.put("etc/resolv.conf", dns.resolvConf(i));
        if (dns != null) files.putAll(dns.files(i, role));
        if (role.equals(ISP_ROUTER)) {
            files.put("router.cfg", dns == null ? ispRouterConfig(i, cells == null ? 0 : cells.coreVillage)
                    : ispRouterConfig(i, cells == null ? 0 : cells.coreVillage, dns.resolver(i), "v" + i + ".ring"));
            files.put("services.cfg", "router on\nsshd &\n" + VillageDiscovery.SERVE + "\n" + (dns == null ? "" : dns.services(i, role)));
            files.put(VillageDiscovery.CONF, VillageDiscovery.ispConf(chatAddress(chat), CHAT_PORT, repo, auth != null));
            files.put("etc/chat.conf", chatConf(chat, "v" + i + "-isp"));
        } else if (role.equals(WEB)) {
            files.put(
                    "network.cfg",
                    "iface eth1 100." + oct + ".0.10/24\nroute default via 100." + oct + ".0.1 dev eth1\n" + dnsLine);
            files.put("services.cfg", "httpd 80 &\nsshd &\n" + (dns == null ? "" : dns.services(i, role)));
            files.put("index.html", website(i, chat, styles, cells, repo, dns));
            files.put("etc/chat.conf", chatConf(chat, "v" + i + "-web"));
        } else if (role.equals(CHAT)) {
            files.put(
                    "network.cfg",
                    "iface eth1 100." + oct + ".0.20/24\nroute default via 100." + oct + ".0.1 dev eth1\n" + dnsLine);
            // The ring's Discord backend; it announces itself to disco serve on its computer (found on the
            // data center LAN), and the ISP routers answer for it ring-wide. With accounts the same
            // computer is the realm (authd) and the music server, and both backends trust the realm.
            if (auth != null) {
                files.put("services.cfg", auth.services(chat) + "sshd &\n" + VillageDiscovery.SERVE + "\n");
                files.put(VillageAuth.KEY_FILE, auth.keyFile());
            } else {
                files.put("services.cfg", discordCommand() + "\nsshd &\n" + VillageDiscovery.SERVE + "\n");
            }
            files.put("etc/chat.conf", chatConf(chat, "v" + i + "-discord"));
        } else if (role.equals(REPO)) {
            // The package server: its repository folder (VillageRepo.serverFiles) is written by provision().
            files.put(
                    "network.cfg",
                    "iface eth1 100." + oct + ".0." + VillageRepo.HOST + "/24\nroute default via 100." + oct + ".0.1 dev eth1\n" + dnsLine);
            files.put("services.cfg", VillageRepo.serveCommand() + "\nsshd &\n" + VillageDiscovery.SERVE + "\n");
            files.put("etc/chat.conf", chatConf(chat, "v" + i + "-repo"));
        } else if (role.endsWith(".router")) {
            files.put("services.cfg", "router on\n" + VillageDiscovery.SERVE + "\n");
            files.put(VillageDiscovery.CONF, VillageDiscovery.RELAY);
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
                            + "dns-server " + (dns == null ? "1.1.1.1" : dns.resolver(i)) + "\n"
                            + (dns == null ? "" : "domain-name v" + i + ".ring\n")
                            + "lease 86400\n"
                            + "enable\n"
                            + "end\n");
            files.put("etc/chat.conf", chatConf(chat, "v" + i + "-" + role.replace(".router", "") + "-rt"));
        } else {
            files.put("network.cfg", "iface eth1 dhcp\n");
            files.put("etc/chat.conf", chatConf(chat, "v" + i + "-" + role.replace(".pc", "")));
            if (role.endsWith(".pc")) files.put("etc/browser.conf", browserConf(i, dns));
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
        return ispRouterConfig(i, 0);
    }

    /**
     * {@link #ispRouterConfig(int)} for a ring with a carrier core in village {@code core} (0:
     * none). The ring links are /26: the ISPs keep .1/.2, taps .3-.14 (the TAPS listen ranges
     * stay the /28), the cell sites along the chord .16 onwards; each village announces the
     * link to its next village so the sites are reachable ring-wide. The core village routes
     * the phones' pool to its core and announces it.
     */
    public static String ispRouterConfig(int i, int core) {
        return ispRouterConfig(i, core, "1.1.1.1", null);
    }

    /**
     * {@link #ispRouterConfig(int, int)} whose DHCP pools hand out {@code dnsServer} and the
     * search domain {@code domain} (null: none): the village resolver and {@code vN.ring}.
     */
    public static String ispRouterConfig(int i, int core, String dnsServer, String domain) {
        String dnsLines = "dns-server " + dnsServer + "\n" + (domain == null ? "" : "domain-name " + domain + "\n");
        int previous = i == 1 ? 10 : i - 1, next = i == 10 ? 1 : i + 1, oct = 64 + i;
        String cfg =
                "configure terminal\nip routing\n"
                        + "interface eth0\nip address 100." + oct + ".1.1/24\nexit\n"
                        + "interface eth1\nip address 100." + oct + ".0.1/24\nexit\n"
                        + "interface eth2\nip address 172.31." + previous + ".2/" + CellPlan.LINK_PREFIX + "\nexit\n"
                        + "interface eth3\nip address 172.31." + i + ".1/" + CellPlan.LINK_PREFIX + "\nexit\n";
        if (i == 1)
            cfg +=
                    "interface eth0\nip nat inside\nexit\n"
                            + "interface eth1\nip nat inside\nexit\n"
                            + "interface eth2\nip nat inside\nexit\n"
                            + "interface eth3\nip nat inside\nexit\n"
                            + "interface eth4\nip address 10.0.0.2/24\nip nat outside\nexit\n"
                            + "ip route 0.0.0.0/0 10.0.0.1 eth4\n";
        if (core == i) cfg += "ip route " + CellPlan.POOL + " " + CellPlan.villageLan(i, CellPlan.CORE_HOST) + " eth1\n";
        cfg +=
                "dhcp-server vrf default\npool village\nrange 100."
                        + oct + ".1.10 100." + oct + ".1.200\ndefault-router 100." + oct + ".1.1\n"
                        + dnsLines + "lease 86400\nenable\nexit\n"
                        + "pool datacenter\nrange 100." + oct + ".0.100 100." + oct + ".0.199\ndefault-router 100."
                        + oct + ".0.1\n" + dnsLines + "lease 86400\nenable\nexit\nexit\n";
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
                        + "network 100." + oct + ".1.0/24\n"
                        + "network 172.31." + i + ".0/" + CellPlan.LINK_PREFIX + "\n";
        if (core == i) cfg += "network " + CellPlan.POOL + "\n";
        if (i == 1) cfg += "network 0.0.0.0/0\n";
        return cfg + "end\n";
    }

    /**
     * Files of the carrier network's computers. The core: its router service (its data center
     * address, forwarding between the LAN and the phones' tun), the database {@code cored init},
     * {@code cored site-add} for every site and {@code cored add} for every Phone Store SIM would
     * write ({@link CellPlan#coreDb}, with every village's number block, the IMS pool and every SIM's number),
 * {@code cored start &} (what {@code cored enable} adds) and the IMS daemons ({@code imsd}, {@code smscd},
 * {@code mmscd}, {@code mrfd} {@code start --wait &}, what their {@code enable} adds).
     * A cell site: its address (the Phone Store's on the data center LAN, a span site's in its
     * chord's link subnet), /etc/celld-site.toml and {@code celld-site start --wait &} (what
     * {@code celld-site enable} adds: the site waits for its SDR, which attaches when its chunk ticks).
     */
    private static void cellConfigs(int i, String role, int chat, CellPlan cells, Map<String, String> files) {
        int oct = 64 + i;
        if (role.equals(CORE)) {
            files.put("router.cfg", "configure terminal\nip routing\ninterface eth1\nip address " + cells.coreAddress()
                    + "/24\nexit\nip route 0.0.0.0/0 100." + oct + ".0.1 eth1\nend\n");
            // Phones' searches reach the core (their gateway); it passes them on to its village's ISP.
            // Telephony (docs/TELEPHONY.md "Tech Villages"): the IMS core beside cored (calls, texts, MMS,
            // voicemail and the 611 support line); each waits for cored's HSS at boot.
            files.put("services.cfg", "router on\ncored start &\nimsd start --wait &\nsmscd start --wait &\nmmscd start --wait &\n"
                    + "mrfd start --wait &\nsshd &\n" + VillageDiscovery.SERVE + "\n");
            files.put(VillageDiscovery.CONF, VillageDiscovery.RELAY);
            files.put("etc/cored.db", cells.coreDb());
            files.put("etc/chat.conf", chatConf(chat, "v" + i + "-core"));
        } else if (role.equals(CellPlan.STORE_ROLE)) {
            files.put("network.cfg", "iface eth1 " + CellPlan.villageLan(i, CellPlan.STORE_HOST) + "/24\nroute default via 100." + oct
                    + ".0.1 dev eth1\n");
            files.put("services.cfg", "celld-site start --wait &\n");
            files.put("etc/celld-site.toml", cells.siteConfig(CellPlan.storeSiteId(i)));
            files.put("etc/chat.conf", chatConf(chat, "v" + i + "-store"));
        } else {
            CellPlan.SpanSite s = cells.span(i - 1, CellPlan.spanK(role));
            if (s == null) throw new IllegalArgumentException("no span site " + role + " on chord " + i);
            files.put("network.cfg", spanNetwork(s, 1));
            files.put("services.cfg", "celld-site start --wait &\n");
            files.put("etc/celld-site.toml", cells.siteConfig(s.id()));
            files.put("etc/chat.conf", chatConf(chat, s.id()));
        }
    }

    /** A span site's network.cfg: its riser reaches eth1 (UP) from a fiber above, eth0 (DOWN) from one below. */
    public static String spanNetwork(CellPlan.SpanSite s, int eth) {
        return "iface eth" + eth + " " + s.address() + "/" + CellPlan.LINK_PREFIX + "\nroute default via " + s.gateway() + " dev eth"
                + eth + "\n";
    }

    /**
     * Provision a span site generated with its riser reaching {@code eth} (network.cfg is written
     * first, so the generated wiring decides; later edits are kept, as on house computers).
     */
    public void provisionSpan(ServerLevel level, CellPlan.SpanSite s, int eth) {
        plan(level);
        Path root = ComputerStorage.path(level.getServer(), identity(level, s.village(), s.role()));
        try {
            Files.createDirectories(root);
            Path net = root.resolve("network.cfg");
            if (!Files.exists(net)) Files.writeString(net, spanNetwork(s, eth));
        } catch (Exception e) {
            throw new IllegalStateException("Cannot provision " + s.id(), e);
        }
        provision(level, s.village(), s.role());
    }

    /** The village website served by its data center (index.html, also at "/"). */
    public static String website(int i, int chat, List<String> styles) {
        return website(i, chat, styles, null);
    }

    /** The village website, with the carrier network's section when there is one. */
    public static String website(int i, int chat, List<String> styles, CellPlan cells) {
        return website(i, chat, styles, cells, null);
    }

    /** The village website, with the Apps section when the world has a package repository. */
    public static String website(int i, int chat, List<String> styles, CellPlan cells, VillageRepo repo) {
        return website(i, chat, styles, cells, repo, null);
    }

    /** The village website, with the DNS section (names, this village's roles) when the ring has DNS. */
    public static String website(int i, int chat, List<String> styles, CellPlan cells, VillageRepo repo, DnsPlan dns) {
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
        String link = cells == null ? "/28" : "/" + CellPlan.LINK_PREFIX;
        html.append("<li>Fiber: eth2 172.31.").append(previous).append(".2").append(link).append(" to village ").append(previous)
                .append(" (AS ").append(65000 + previous).append("), eth3 172.31.").append(i).append(".1").append(link).append(" to village ")
                .append(next).append(" (AS ").append(65000 + next).append(")</li>\n");
        html.append("<li>Open peering on both fiber links: tap a span with a patch panel, take a free address .3-.14 in")
                .append(" that link and peer from any AS (eBGP; up to 20 of your own prefixes, /24 or shorter)")
                .append(cells == null ? "" : "; the carrier's cell sites along the fiber use .16 up").append("</li>\n");
        if (i == 1) html.append("<li>Uplink: eth4 10.0.0.2/24 to the internet gateway 10.0.0.1 (NAT)</li>\n");
        html.append("<li>Spare ports for peering: eth5-eth8</li>\n</ul>\n");
        html.append("<h2>Discord</h2>\n<p>The ring's Discord backend runs in the data center of village ").append(chat)
                .append(": ").append(chatAddress(chat)).append(" port ").append(CHAT_PORT).append(" (voice on UDP ").append(CHAT_PORT + 1)
                .append("). Install Discord from the App Store: it finds this backend by itself, and you can make your own servers with")
                .append(" text and voice channels there.</p>\n")
                .append("<p>Sign in with a Tech Village account: the first time, Discord shows the realm \"").append(VillageAuth.NAME)
                .append("\" (already trusted on village computers and Phone Store phones) and you create an account there. The same")
                .append(" account works in Music. From a console: <code>chat --user NAME --new</code> once, then <code>chat</code>")
                .append(" (village computers read /etc/chat.conf) to talk in the home server's #general; /who, /quit.</p>\n")
                .append("<h2>Music</h2>\n<p>").append(VillageAuth.MUSIC_NAME).append(" streams from the same data center: ")
                .append(chatAddress(chat)).append(" port ").append(VillageAuth.MUSIC_PORT).append(". Install Music from the App Store,")
                .append(" sign in with your Tech Village account, search, make playlists, and keep listening with the phone in your")
                .append(" pocket. The songs are listed in the mod's docs/MUSIC_CREDITS.md (all CC0).</p>\n");
        if (repo != null)
            html.append("<h2>Apps</h2>\n<p>The Tech Village app repository, ").append(VillageRepo.NAME).append(", runs in the data center of village ")
                    .append(repo.village()).append(": ").append(repo.url()).append(". The App Store on every village computer and every")
                    .append(" Phone Store phone already lists it (Discord is there, from a verified publisher). On your own computer: <code>pkg repo add ")
                    .append(VillageRepo.NAME).append(" ").append(repo.url()).append(" --key ").append(repo.publicKey()).append("</code></p>\n");
        if (cells != null) {
            html.append("<h2>Cellular</h2>\n<p>").append(cells.displayName()).append(" (carrier <code>").append(cells.carrier)
                    .append("</code>) covers every Tech Village and the fiber between them on LTE channel ").append(cells.channel)
                    .append(". Take a phone or a SIM card from the Phone Store next to this data center: the SIMs are already on the network.")
                    .append(" Phones get an address in ").append(CellPlan.POOL).append(" from the carrier core in village ")
                    .append(cells.coreVillage).append(" (").append(cells.coreAddress()).append(") and keep it from cell to cell.</p>\n");
        }
        if (dns != null) {
            html.append("<h2>Names (DNS)</h2>\n<p>This village is <code>v").append(i).append(".ring</code>: this site is <a href=\"http://web.v")
                    .append(i).append(".ring/\">http://web.v").append(i).append(".ring/</a> (or just <code>http://web/</code> from a computer")
                    .append(" in the village: DHCP hands out the search domain v").append(i).append(".ring). The village resolver is ")
                    .append(dns.resolver(i)).append(". Register your own <code>.ecm</code> name at <a href=\"http://nic.ring/\">http://nic.ring/</a>")
                    .append(" or with <code>domain register NAME.ecm</code>; <code>dig +trace NAME</code> shows how a name is found.</p>\n");
            html.append("<p>This village's part of DNS:</p>\n<ul>\n");
            for (String r : dns.describe(i)) html.append("<li>").append(r).append("</li>\n");
            html.append("</ul>\n");
        }
        html.append("<h2>Other villages</h2>\n<ul>\n");
        for (int v = 1; v <= 10; v++) {
            if (v == i) continue;
            html.append("<li><a href=\"").append(dns == null ? "http://100." + (64 + v) + ".0.10/" : "http://web.v" + v + ".ring/")
                    .append("\">Tech Village ").append(v)
                    .append("</a> (").append(v <= styles.size() ? styles.get(v - 1) : "?").append(", AS ")
                    .append(65000 + v).append(")</li>\n");
        }
        html.append("</ul>\n</body></html>\n");
        return html.toString();
    }
}
//?}
