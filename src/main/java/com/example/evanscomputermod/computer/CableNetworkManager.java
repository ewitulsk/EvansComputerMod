package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.InterfaceBlock;
import com.example.evanscomputermod.block.InternetGatewayBlock;
import com.example.evanscomputermod.block.NetworkCableBlock;
import com.example.evanscomputermod.block.TerminalBlock;
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

    private final AtomicInteger nextNetworkId = new AtomicInteger(0);

    // ===== Lifecycle =====

    public static void init(MinecraftServer server) {
        INSTANCE = new CableNetworkManager();
        INSTANCE.server = server;
        EvansComputerMod.LOGGER.info("CableNetworkManager initialized");
    }

    public static void shutdown() {
        if (INSTANCE != null) {
            INSTANCE.macToPos.clear();
            INSTANCE.macToLevel.clear();
            INSTANCE.macToNetworkId = new ConcurrentHashMap<>();
            INSTANCE.internetNetworkIds = ConcurrentHashMap.newKeySet();
            INSTANCE = null;
            EvansComputerMod.LOGGER.info("CableNetworkManager shut down");
        }
    }

    public static CableNetworkManager getInstance() {
        return INSTANCE;
    }

    // ===== Terminal Registration =====

    public void registerTerminal(BlockPos terminalPos, ResourceKey<Level> dimension, byte[][] macs, BlockPos[] exitPositions) {
        for (int i = 0; i < macs.length; i++) {
            MacAddress key = new MacAddress(macs[i]);
            macToPos.put(key, exitPositions[i]);  // Use EXIT position, not terminal position
            macToLevel.put(key, dimension);
        }
        EvansComputerMod.LOGGER.debug("CableNetworkManager: registered {} interfaces for terminal at {}",
                macs.length, terminalPos);
        recomputeNetworks();
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
            if (!level.isLoaded(exitPos)) continue;
            Block exitBlock = level.getBlockState(exitPos).getBlock();
            if (!isNetworkBlock(exitBlock)) {
                // No cable at this face — MAC is isolated
                continue;
            }

            int networkId = nextNetworkId.getAndIncrement();
            boolean hasGateway = bfsFromExit(level, exitPos, networkId, newMacToNetwork, visited);
            if (hasGateway) {
                newInternetNetworks.add(networkId);
            }
        }

        // Atomically swap the maps
        macToNetworkId = newMacToNetwork;
        internetNetworkIds = newInternetNetworks;
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
            Block block = level.getBlockState(current).getBlock();

            // Check if this is the internet gateway
            if (block instanceof InternetGatewayBlock) {
                foundGateway = true;
            }

            // Check if any registered MAC has this as its exit position
            for (Map.Entry<MacAddress, BlockPos> entry : macToPos.entrySet()) {
                if (entry.getValue().equals(current)) {
                    macToNetwork.put(entry.getKey(), networkId);
                    visitedMacs.add(entry.getKey());
                }
            }

            // Explore 6 neighbors
            for (BlockPos neighbor : getNeighbors(current)) {
                if (visitedPositions.contains(neighbor)) continue;
                if (!level.isLoaded(neighbor)) continue;

                Block neighborBlock = level.getBlockState(neighbor).getBlock();
                if (isNetworkBlock(neighborBlock)) {
                    visitedPositions.add(neighbor);
                    queue.add(neighbor);
                }
            }
        }

        return foundGateway;
    }

    /**
     * Returns true if this block type participates in cable network BFS.
     */
    private static boolean isNetworkBlock(Block block) {
        return block instanceof NetworkCableBlock
                || block instanceof TerminalBlock
                || block instanceof InternetGatewayBlock
                || block instanceof InterfaceBlock;
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
    static class MacAddress {
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
