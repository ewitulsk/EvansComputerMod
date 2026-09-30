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
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Manages cable network topology. Determines which computers are on the same
 * physical cable network by doing BFS through cable/terminal/gateway blocks.
 *
 * Thread safety: recomputeNetworks() runs on the server thread.
 * areOnSameNetwork() and hasInternetAccess() read from pre-computed concurrent maps
 * and are safe to call from WASM worker threads.
 */
public class CableNetworkManager {
    private static CableNetworkManager INSTANCE;

    private MinecraftServer server;

    // Registered terminals: MAC -> position and dimension
    private final Map<MacAddress, BlockPos> macToPos = new ConcurrentHashMap<>();
    private final Map<MacAddress, ResourceKey<Level>> macToLevel = new ConcurrentHashMap<>();

    // Pre-computed network assignments (written on server thread, read from worker threads)
    private volatile Map<MacAddress, Integer> macToNetworkId = new ConcurrentHashMap<>();
    private volatile Set<Integer> internetNetworkIds = ConcurrentHashMap.newKeySet();
    /** networkId -> member MACs (immutable snapshot, rebuilt with macToNetworkId). */
    private volatile Map<Integer, List<MacAddress>> networkMembers = Map.of();

    private final AtomicInteger nextNetworkId = new AtomicInteger(0);
    private final Map<String,List<MacAddress>> logicalLinks=new HashMap<>();
    private final Map<String,List<MacAddress>> failedLinks=new HashMap<>();
    private volatile Set<MacAddress> failedPorts=Set.of();
    private final Map<String,Integer> knownBlocks=new HashMap<>();
    public void logicalLink(String name,byte[] a,byte[] b,boolean intact) {
        if(intact) {logicalLinks.put(name,List.of(new MacAddress(a),new MacAddress(b)));failedLinks.remove(name);}
        else {logicalLinks.remove(name);failedLinks.put(name,List.of(new MacAddress(a),new MacAddress(b)));}
        recomputeNetworks();
    }
    private String blockKey(ServerLevel level,BlockPos pos) {
        //? if >=26.1 {
        return level.dimension().identifier()+"/"+pos.asLong();
        //?} else {
        /*return level.dimension().location()+"/"+pos.asLong();*/
        //?}
    }
    private int networkBlock(ServerLevel level,BlockPos pos) {
        String key=blockKey(level,pos);
        if(!level.isLoaded(pos)) return knownBlocks.getOrDefault(key,0);
        Block b=level.getBlockState(pos).getBlock();int value=b instanceof InternetGatewayBlock?2:isNetworkBlock(b)?1:0;
        if(value==0) knownBlocks.remove(key);else knownBlocks.put(key,value);return value;
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
            INSTANCE.macToNetworkId = new ConcurrentHashMap<>();
            INSTANCE.internetNetworkIds = ConcurrentHashMap.newKeySet();
            INSTANCE.networkMembers = Map.of();
            INSTANCE = null;
            EvansComputerMod.LOGGER.info("CableNetworkManager shut down");
        }
    }

    public static CableNetworkManager getInstance() {
        return INSTANCE;
    }

    // ===== Terminal Registration =====

    public void registerTerminal(BlockPos terminalPos, ResourceKey<Level> dimension, byte[][] macs, BlockPos[] exitPositions) {
        for (int i = 0; i < Math.min(macs.length,exitPositions.length); i++) {
            MacAddress key = new MacAddress(macs[i]);
            macToPos.put(key, exitPositions[i]);  // Use EXIT position, not terminal position
            macToLevel.put(key, dimension);
        }
        EvansComputerMod.LOGGER.info("CableNetworkManager: registered {} interfaces for terminal at {} (dim={})",
                macs.length, terminalPos, dimension);
        recomputeNetworks();
        EvansComputerMod.LOGGER.info("CableNetworkManager: post-recompute, {} MACs assigned to networks",
                macToNetworkId.size());
    }

    public void unregisterTerminal(byte[][] macs) {
        for (byte[] mac : macs) {
            MacAddress key = new MacAddress(mac);
            macToPos.remove(key);
            macToLevel.remove(key);
        }
        recomputeNetworks();
    }

    // ===== Cache Invalidation =====

    /**
     * Called from block place/break events (always on server thread).
     * Eagerly recomputes all network assignments.
     */
    public void invalidateCache() {
        recomputeNetworks();
    }

    // ===== Queries (safe from any thread) =====

    /**
     * Check if two MACs are on the same physical cable network.
     */
    public boolean areOnSameNetwork(byte[] macA, byte[] macB) {
        Map<MacAddress, Integer> snapshot = macToNetworkId;
        Integer netA = snapshot.get(new MacAddress(macA));
        Integer netB = snapshot.get(new MacAddress(macB));
        if (netA == null || netB == null) return false;
        return netA.equals(netB);
    }

    /** The segment (network id) a NIC's face is cabled into, or null. */
    public Integer networkOf(byte[] mac) {
        return macToNetworkId.get(new MacAddress(mac));
    }
    public boolean carrierOf(byte[] mac) {return networkOf(mac)!=null && !failedPorts.contains(new MacAddress(mac));}

    /** All NICs on a segment. */
    public List<MacAddress> membersOf(int networkId) {
        return networkMembers.getOrDefault(networkId, List.of());
    }

    /**
     * Check if a MAC is on a cable network that reaches the Internet Gateway.
     */
    public boolean hasInternetAccess(byte[] mac) {
        Map<MacAddress, Integer> snapshot = macToNetworkId;
        Integer netId = snapshot.get(new MacAddress(mac));
        if (netId == null) return false;
        return internetNetworkIds.contains(netId);
    }

    // ===== BFS Network Computation =====

    /**
     * Recompute all network assignments by doing BFS from each MAC's exit position.
     * Must run on the server thread (reads world block state).
     */
    private void recomputeNetworks() {
        if (server == null) return;

        Map<MacAddress, Integer> newMacToNetwork = new ConcurrentHashMap<>();
        Set<Integer> newInternetNetworks = ConcurrentHashMap.newKeySet();
        Set<MacAddress> visited = new HashSet<>();
        nextNetworkId.set(0);

        for (Map.Entry<MacAddress, BlockPos> entry : macToPos.entrySet()) {
            MacAddress mac = entry.getKey();
            if (visited.contains(mac)) continue;

            BlockPos exitPos = entry.getValue();
            ResourceKey<Level> dimKey = macToLevel.get(mac);
            if (dimKey == null) continue;

            ServerLevel level = server.getLevel(dimKey);
            if (level == null) continue;

            // Check if exit position has a network block
            if (networkBlock(level,exitPos)==0) {
                // No cable at this face — MAC is isolated
                continue;
            }

            int networkId = nextNetworkId.getAndIncrement();
            boolean hasGateway = bfsFromExit(level, exitPos, networkId, newMacToNetwork, visited);
            if (hasGateway) {
                newInternetNetworks.add(networkId);
            }
        }

        for(List<MacAddress> link:logicalLinks.values()) {
            MacAddress a=link.get(0),b=link.get(1);Integer ai=newMacToNetwork.get(a),bi=newMacToNetwork.get(b);
            int joined=ai!=null?ai:bi!=null?bi:nextNetworkId.getAndIncrement();
            if(ai!=null && bi!=null && !ai.equals(bi)) {
                int from=bi;newMacToNetwork.replaceAll((mac,id)->id==from?joined:id);
                if(newInternetNetworks.remove(from)) newInternetNetworks.add(joined);
            }
            newMacToNetwork.put(a,joined);newMacToNetwork.put(b,joined);
        }
        for(var link:logicalLinks.entrySet()) if(link.getKey().startsWith("internet-")) {
            Integer network=newMacToNetwork.get(link.getValue().get(0));if(network!=null) newInternetNetworks.add(network);
        }
        //? if <=1.21.1 {
        var saved=WorldNetwork.get(server.overworld());
        if(!saved.cableBlocks.equals(knownBlocks)) {saved.cableBlocks.clear();saved.cableBlocks.putAll(knownBlocks);saved.setDirty();}
        Map<String,long[]> positions=new HashMap<>();
        macToPos.forEach((mac,pos)->{var dim=macToLevel.get(mac);if(dim!=null) positions.put(dim.location()+"/"+java.util.HexFormat.of().formatHex(mac.bytes),new long[]{pos.asLong()});});
        saved.nicPositions.clear();saved.nicPositions.putAll(positions);saved.setDirty();
        //?}
        Map<Integer, List<MacAddress>> members = new HashMap<>();
        for (Map.Entry<MacAddress, Integer> e : newMacToNetwork.entrySet()) {
            members.computeIfAbsent(e.getValue(), k -> new ArrayList<>()).add(e.getKey());
        }
        Map<Integer, List<MacAddress>> frozen = new HashMap<>();
        members.forEach((k, v) -> frozen.put(k, List.copyOf(v)));

        // Atomically swap the maps
        macToNetworkId = newMacToNetwork;
        internetNetworkIds = newInternetNetworks;
        networkMembers = Map.copyOf(frozen);
        Set<MacAddress> failed=new HashSet<>();failedLinks.values().forEach(failed::addAll);failedPorts=Set.copyOf(failed);
    }

    /**
     * BFS from an exit position through cable/terminal/gateway/interface blocks.
     * Returns true if the BFS reached an InternetGatewayBlock.
     */
    private boolean bfsFromExit(ServerLevel level, BlockPos start, int networkId,
                                Map<MacAddress, Integer> macToNetwork, Set<MacAddress> visitedMacs) {
        Set<BlockPos> visitedPositions = new HashSet<>();
        Queue<BlockPos> queue = new LinkedList<>();
        boolean foundGateway = false;

        queue.add(start);
        visitedPositions.add(start);

        while (!queue.isEmpty()) {
            BlockPos current = queue.poll();

            // Check if this is the internet gateway
            if (networkBlock(level,current)==2) {
                foundGateway = true;
            }

            // Check if any registered MAC has this as its exit position
            for (Map.Entry<MacAddress, BlockPos> entry : macToPos.entrySet()) {
                if (entry.getValue().equals(current) && level.dimension().equals(macToLevel.get(entry.getKey()))) {
                    macToNetwork.put(entry.getKey(), networkId);
                    visitedMacs.add(entry.getKey());
                }
            }

            // Explore 6 neighbors
            for (BlockPos neighbor : getNeighbors(current)) {
                if (visitedPositions.contains(neighbor)) continue;
                if (networkBlock(level,neighbor)!=0) {
                    visitedPositions.add(neighbor);
                    queue.add(neighbor);
                }
            }
        }

        return foundGateway;
    }

    /**
     * Returns true if this block type participates in cable network BFS.
     *
     * Terminal and Interface blocks are intentionally excluded: each of their
     * faces hosts a separate NIC whose cable mesh should be its own network.
     * If BFS walked through a terminal block, all faces (and therefore all
     * NICs of that computer) would collapse into one super-network, which
     * defeats the purpose of having multiple NICs and turns an L2 switch
     * built on a terminal into a self-looping broadcast amplifier — every
     * frame the switch forwards out one port comes back in on every other
     * port via the shared network's promiscuous delivery.
     *
     * NIC exit positions are the cable block (or air) adjacent to a terminal
     * face, not the terminal itself, so MACs are still assigned correctly:
     * BFS visits the cable, sees the cable equals the NIC's exit position,
     * and adds the MAC to the current network.
     */
    private static boolean isNetworkBlock(Block block) {
        return block instanceof NetworkCableBlock
                || block instanceof InternetGatewayBlock;
    }

    private static List<BlockPos> getNeighbors(BlockPos pos) {
        return List.of(
                pos.above(), pos.below(),
                pos.north(), pos.south(),
                pos.east(), pos.west()
        );
    }

    /**
     * Reuse NetworkHub's MacAddress wrapper for map keys.
     */
    public static class MacAddress {
        final byte[] bytes;
        final int hash;

        MacAddress(byte[] mac) {
            this.bytes = mac.clone();
            this.hash = Arrays.hashCode(bytes);
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
    }
}
