package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.InternetGatewayBlock;
import com.example.evanscomputermod.block.NetworkCableBlock;
import net.minecraft.core.BlockPos;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;

import java.util.*;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Cable network topology: which NICs share a wire ({@link SegmentGraph}).
 *
 * <p>Changes (blocks placed or broken, terminals registered, logical links) only mark the
 * topology dirty; it is recomputed at most once per server tick, or at once when a query
 * comes from the server thread while it is dirty (so server-side callers always see the
 * current state). Worker threads read the last computed snapshot.
 *
 * <p>Blocks in unloaded chunks are read from the last-known snapshot (saved with the
 * world); never-generated blocks do not conduct. The generated fiber ring is supplied by
 * a {@link Topology} (Tech Villages, 1.21.1) as pieces with attached panels, plus
 * stand-ins for buildings that do not exist yet.
 */
public class CableNetworkManager {
    private static CableNetworkManager INSTANCE;

    private MinecraftServer server;

    // Registered NICs: MAC -> exit position (the block on the face) and dimension
    private final Map<MacAddress, BlockPos> macToPos = new ConcurrentHashMap<>();
    private final Map<MacAddress, ResourceKey<Level>> macToLevel = new ConcurrentHashMap<>();

    // Snapshot (written on the server thread, read from worker threads)
    private volatile Map<MacAddress, Integer> macToNetworkId = Map.of();
    private volatile Set<Integer> internetNetworkIds = Set.of();
    private volatile Map<Integer, List<MacAddress>> networkMembers = Map.of();
    private volatile Set<MacAddress> carrier = Set.of();
    private volatile SegmentGraph.Result<MacAddress> last;

    private final Map<String, List<MacAddress>> logicalLinks = new HashMap<>();
    private final Map<String, List<MacAddress>> failedLinks = new HashMap<>();
    private volatile Set<MacAddress> failedPorts = Set.of();
    private final Map<String, Integer> knownBlocks = new HashMap<>();
    private final List<ResourceKey<Level>> dims = new ArrayList<>();
    private volatile Topology topology;
    private volatile boolean dirty = true;

    /** Recompute statistics: count, last and worst duration (ns), blocks read last time. */
    private long recomputes, lastNanos, maxNanos;
    private int lastBlocks;

    /** World-level topology: the generated ring and stand-ins (set by WorldNetwork). */
    public interface Topology {
        /**
         * The ring's pieces in dimension index {@code dim} (the overworld), or null. Called on
         * the server thread before each recompute.
         */
        SegmentGraph.Ring ring(int dim);

        /** Stand-ins and extra logical edges for buildings that do not exist yet. */
        void standIns(List<SegmentGraph.StandIn<MacAddress>> standIns, List<SegmentGraph.Edge<MacAddress>> edges);

        /** Called after each recompute with the result (on the server thread). */
        default void computed(SegmentGraph.Result<MacAddress> result) {}
    }

    public void setTopology(Topology topology) {
        this.topology = topology;
        invalidateCache();
    }

    /** Block codes of the topology (also the values of the saved last-known snapshot). */
    public static final int NONE = SegmentGraph.NONE, CABLE = SegmentGraph.CABLE, GATEWAY = SegmentGraph.GATEWAY,
            FIBER = SegmentGraph.FIBER, PANEL = SegmentGraph.PANEL;

    /**
     * A logical link (lab lead, internet uplink): the two NICs share a segment while
     * {@code intact}; a link registered as not intact holds both NICs' carrier down.
     */
    public synchronized void logicalLink(String name, byte[] a, byte[] b, boolean intact) {
        if (intact) {
            logicalLinks.put(name, List.of(new MacAddress(a), new MacAddress(b)));
            failedLinks.remove(name);
        } else {
            logicalLinks.remove(name);
            failedLinks.put(name, List.of(new MacAddress(a), new MacAddress(b)));
        }
        invalidateCache();
    }

    /** Do two neighbouring blocks conduct? See {@link SegmentGraph#joins}. */
    public static boolean joins(int a, int b) {
        return SegmentGraph.joins(a, b);
    }

    private String blockKey(ServerLevel level, BlockPos pos) {
        //? if >=26.1 {
        return level.dimension().identifier()+"/"+pos.asLong();
        //?} else {
        /*return level.dimension().location()+"/"+pos.asLong();*/
        //?}
    }

    /** Block code at a position: the world where loaded, else the last-known snapshot. */
    public int networkBlock(ServerLevel level, BlockPos pos) {
        String key = blockKey(level, pos);
        if (!level.isLoaded(pos)) return knownBlocks.getOrDefault(key, NONE);
        Block b = level.getBlockState(pos).getBlock();
        int value = b instanceof InternetGatewayBlock ? GATEWAY
                : b instanceof com.example.evanscomputermod.block.FiberPatchPanelBlock ? PANEL
                : b instanceof NetworkCableBlock ? CABLE
                : b instanceof com.example.evanscomputermod.block.FiberInfrastructureBlock ? FIBER
                : NONE;
        if (value == NONE) knownBlocks.remove(key);
        else knownBlocks.put(key, value);
        return value;
    }

    // ===== Lifecycle =====

    public static void init(MinecraftServer server) {
        INSTANCE = new CableNetworkManager();
        INSTANCE.server = server;
        //? if <=1.21.1 {
        var data=WorldNetwork.get(server.overworld());INSTANCE.knownBlocks.putAll(data.cableBlocks);
        for(var e:data.nicPositions.entrySet()) {
            String[] parts=e.getKey().split("/",2);if(parts.length!=2 || e.getValue().length!=1) continue;
            byte[] bytes;try {bytes=java.util.HexFormat.of().parseHex(parts[1]);}catch(Exception bad) {continue;}
            if(bytes.length!=6) continue;
            var dim=ResourceKey.create(net.minecraft.core.registries.Registries.DIMENSION,net.minecraft.resources.ResourceLocation.parse(parts[0]));
            MacAddress mac=new MacAddress(bytes);INSTANCE.macToPos.put(mac,BlockPos.of(e.getValue()[0]));INSTANCE.macToLevel.put(mac,dim);
        }
        //?}
        EvansComputerMod.LOGGER.info("CableNetworkManager initialized");
    }

    public static void shutdown() {
        if (INSTANCE != null) {
            INSTANCE.macToPos.clear();
            INSTANCE.macToLevel.clear();
            INSTANCE.macToNetworkId = Map.of();
            INSTANCE.internetNetworkIds = Set.of();
            INSTANCE.networkMembers = Map.of();
            INSTANCE.carrier = Set.of();
            INSTANCE = null;
            EvansComputerMod.LOGGER.info("CableNetworkManager shut down");
        }
    }

    public static CableNetworkManager getInstance() {
        return INSTANCE;
    }

    /** Server tick: apply pending topology changes (at most one recompute per tick). */
    public static void tick() {
        CableNetworkManager m = INSTANCE;
        if (m != null) m.flush();
    }

    // ===== Terminal Registration =====

    public void registerTerminal(BlockPos terminalPos, ResourceKey<Level> dimension, byte[][] macs, BlockPos[] exitPositions) {
        for (int i = 0; i < Math.min(macs.length, exitPositions.length); i++) {
            MacAddress key = new MacAddress(macs[i]);
            macToPos.put(key, exitPositions[i]); // the EXIT position, not the terminal's
            macToLevel.put(key, dimension);
        }
        EvansComputerMod.LOGGER.debug("CableNetworkManager: registered {} interfaces for terminal at {} (dim={})",
                macs.length, terminalPos, dimension);
        invalidateCache();
    }

    public void unregisterTerminal(byte[][] macs) {
        for (byte[] mac : macs) {
            MacAddress key = new MacAddress(mac);
            macToPos.remove(key);
            macToLevel.remove(key);
        }
        invalidateCache();
    }

    // ===== Cache Invalidation =====

    /** Topology changed (block placed/broken, link changed): recompute before the next read. */
    public void invalidateCache() {
        dirty = true;
    }

    /** Recompute now if anything changed (server thread only; otherwise a no-op). */
    public void flush() {
        if (dirty && server != null && server.isSameThread()) recomputeNetworks();
    }

    private void fresh() {
        if (dirty) flush();
    }

    // ===== Queries (safe from any thread; the server thread sees pending changes) =====

    /** Check if two MACs are on the same physical cable network. */
    public boolean areOnSameNetwork(byte[] macA, byte[] macB) {
        fresh();
        Map<MacAddress, Integer> snapshot = macToNetworkId;
        Integer netA = snapshot.get(new MacAddress(macA));
        Integer netB = snapshot.get(new MacAddress(macB));
        if (netA == null || netB == null) return false;
        return netA.equals(netB);
    }

    /** The segment (network id) a NIC's face is cabled into, or null. */
    public Integer networkOf(byte[] mac) {
        fresh();
        return macToNetworkId.get(new MacAddress(mac));
    }

    /** Remove a temporary lab lead without leaving a failed-carrier override. */
    public synchronized void removeLogicalLink(String key) {
        logicalLinks.remove(key);
        failedLinks.remove(key);
        invalidateCache();
    }

    /**
     * Physical link: the NIC is on a segment with a partner on it (another NIC or the
     * internet gateway) and no failed lab lead holds it down. A cable or fiber that ends
     * nowhere, or a fiber cut between this NIC and everyone else, has no link.
     */
    public boolean carrierOf(byte[] mac) {
        fresh();
        MacAddress m = new MacAddress(mac);
        return carrier.contains(m) && !failedPorts.contains(m);
    }

    /** A NIC's registered exit position (the block on its face), or null. */
    public BlockPos exitOf(byte[] mac) {
        return macToPos.get(new MacAddress(mac));
    }

    /** All NICs on a segment. */
    public List<MacAddress> membersOf(int networkId) {
        return networkMembers.getOrDefault(networkId, List.of());
    }

    /** Check if a MAC is on a cable network that reaches the Internet Gateway. */
    public boolean hasInternetAccess(byte[] mac) {
        Integer netId = macToNetworkId.get(new MacAddress(mac));
        if (netId == null) return false;
        return internetNetworkIds.contains(netId);
    }

    /** The last computed result (server thread: current), for diagnostics. */
    public SegmentGraph.Result<MacAddress> result() {
        fresh();
        return last;
    }

    /** "N recomputes, last X ms (B blocks), worst Y ms". */
    public synchronized String stats() {
        return String.format(Locale.ROOT, "%d recomputes, last %.2f ms (%d blocks read), worst %.2f ms", recomputes,
                lastNanos / 1e6, lastBlocks, maxNanos / 1e6);
    }

    public synchronized long lastRecomputeNanos() {
        return lastNanos;
    }

    private int dimIndex(ResourceKey<Level> key) {
        int i = dims.indexOf(key);
        if (i >= 0) return i;
        dims.add(key);
        return dims.size() - 1;
    }

    // ===== Segment computation =====

    /** Recompute every segment. Server thread (reads block state). */
    private synchronized void recomputeNetworks() {
        if (server == null) return;
        dirty = false;
        long t0 = System.nanoTime();
        List<SegmentGraph.Exit<MacAddress>> exits = new ArrayList<>();
        macToPos.forEach((mac, pos) -> {
            ResourceKey<Level> dim = macToLevel.get(mac);
            if (dim != null && server.getLevel(dim) != null) exits.add(new SegmentGraph.Exit<>(mac, dimIndex(dim), pos.asLong()));
        });
        List<SegmentGraph.Edge<MacAddress>> edges = new ArrayList<>();
        logicalLinks.forEach((name, l) -> edges.add(new SegmentGraph.Edge<>(l.get(0), l.get(1), name.startsWith("internet-"))));
        List<SegmentGraph.StandIn<MacAddress>> standIns = new ArrayList<>();
        Topology topo = topology;
        SegmentGraph.Ring ring = null;
        int overworld = dimIndex(Level.OVERWORLD);
        if (topo != null)
            try {
                ring = topo.ring(overworld);
                topo.standIns(standIns, edges);
            } catch (RuntimeException e) {
                EvansComputerMod.LOGGER.error("Ring topology failed", e);
                ring = null;
            }
        SegmentGraph.Blocks blocks = (dim, pos) -> {
            ServerLevel level = server.getLevel(dims.get(dim));
            return level == null ? NONE : networkBlock(level, BlockPos.of(pos));
        };
        SegmentGraph.Result<MacAddress> r = SegmentGraph.compute(exits, blocks, ring, edges, standIns);

        //? if <=1.21.1 {
        var saved=WorldNetwork.get(server.overworld());
        if(!saved.cableBlocks.equals(knownBlocks)) {saved.cableBlocks.clear();saved.cableBlocks.putAll(knownBlocks);saved.setDirty();}
        Map<String,long[]> positions=new HashMap<>();
        macToPos.forEach((mac,pos)->{var dim=macToLevel.get(mac);if(dim!=null) positions.put(dim.location()+"/"+java.util.HexFormat.of().formatHex(mac.bytes),new long[]{pos.asLong()});});
        if(!sameNics(saved.nicPositions,positions)) {saved.nicPositions.clear();saved.nicPositions.putAll(positions);saved.setDirty();}
        //?}

        Set<MacAddress> up = new HashSet<>();
        for (MacAddress m : r.segment.keySet()) if (r.carrier(m)) up.add(m);
        Set<MacAddress> failed = new HashSet<>();
        failedLinks.values().forEach(failed::addAll);
        macToNetworkId = Map.copyOf(r.segment);
        internetNetworkIds = Set.copyOf(r.internet);
        networkMembers = Map.copyOf(r.members);
        carrier = Set.copyOf(up);
        failedPorts = Set.copyOf(failed);
        last = r;
        long dt = System.nanoTime() - t0;
        recomputes++;
        lastNanos = dt;
        maxNanos = Math.max(maxNanos, dt);
        lastBlocks = r.blocksVisited;
        if (topo != null)
            try {
                topo.computed(r);
            } catch (RuntimeException e) {
                EvansComputerMod.LOGGER.error("Ring topology callback failed", e);
            }
    }

    private static boolean sameNics(Map<String, long[]> a, Map<String, long[]> b) {
        if (a.size() != b.size()) return false;
        for (var e : b.entrySet()) {
            long[] o = a.get(e.getKey());
            if (o == null || !Arrays.equals(o, e.getValue())) return false;
        }
        return true;
    }

    /*
     * Terminal and Interface blocks are not part of a segment: each of their faces hosts a
     * separate NIC whose cable mesh is its own network (otherwise every NIC of a computer
     * would collapse into one segment, and an L2 switch built on a terminal would loop
     * frames back through itself). NIC exit positions are the block adjacent to a face.
     */

    /** MAC address as a map key. */
    public static class MacAddress {
        final byte[] bytes;
        final int hash;

        public MacAddress(byte[] mac) {
            this.bytes = mac.clone();
            this.hash = Arrays.hashCode(bytes);
        }

        public byte[] bytes() {
            return bytes.clone();
        }

        @Override
        public boolean equals(Object o) {
            if (this == o) return true;
            if (!(o instanceof MacAddress)) return false;
            return Arrays.equals(bytes, ((MacAddress) o).bytes);
        }

        @Override
        public int hashCode() {
            return hash;
        }

        @Override
        public String toString() {
            return java.util.HexFormat.ofDelimiter(":").formatHex(bytes);
        }
    }
}
