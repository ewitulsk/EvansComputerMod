package com.example.evanscomputermod.sensor.wire;

//? if <=1.21.1 {
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.Level;

import java.util.HashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.WeakHashMap;

/**
 * Server-side index of which wire entities end at which endpoint (a block's
 * connection point or a junction). It replaces the connection map PowerGrid
 * keeps in each block's {@code ElectricBehaviour}: it isn't saved, and is
 * rebuilt as wire entities load and resolve their endpoints.
 */
public final class WireConnections {
    private static final Map<Level, Map<IWireEndpoint, Set<BaseWireEntity>>> LEVELS = new WeakHashMap<>();

    private WireConnections() {
    }

    private static Map<IWireEndpoint, Set<BaseWireEntity>> of(Level level) {
        return LEVELS.computeIfAbsent(level, l -> new HashMap<>());
    }

    public static synchronized void add(Level level, IWireEndpoint endpoint, BaseWireEntity wire) {
        var map = of(level);
        var set = map.computeIfAbsent(endpoint, e -> new LinkedHashSet<>());
        set.removeIf(BaseWireEntity::isRemoved);
        set.add(wire);
    }

    public static synchronized void remove(Level level, IWireEndpoint endpoint, BaseWireEntity wire) {
        var map = of(level);
        var set = map.get(endpoint);
        if(set == null)
            return;
        set.remove(wire);
        if(set.isEmpty())
            map.remove(endpoint);
    }

    public static synchronized void clear(Level level, IWireEndpoint endpoint) {
        var map = LEVELS.get(level);
        if(map != null)
            map.remove(endpoint);
    }

    /** Live wires ending at {@code endpoint} (a snapshot). */
    public static synchronized List<BaseWireEntity> get(Level level, IWireEndpoint endpoint) {
        var map = LEVELS.get(level);
        if(map == null)
            return List.of();
        var set = map.get(endpoint);
        if(set == null)
            return List.of();
        set.removeIf(BaseWireEntity::isRemoved);
        return List.copyOf(set);
    }

    public static boolean hasConnection(Level level, IWireEndpoint endpoint) {
        return !get(level, endpoint).isEmpty();
    }

    /** The host block at {@code pos} went away: every wire on its terminals loses that end. */
    public static void breakAll(Level level, BlockPos pos, int terminalCount) {
        if(level.isClientSide())
            return;
        for(int i = 0; i < terminalCount; ++i) {
            breakTerminal(level, pos, i);
        }
    }

    public static void breakTerminal(Level level, BlockPos pos, int terminal) {
        var endpoint = new BlockWireEndpoint(pos, terminal);
        for(var wire : get(level, endpoint)) {
            wire.endpointRemoved(endpoint);
        }
    }
}
//?}
