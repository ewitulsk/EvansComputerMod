package com.example.evanscomputermod.sensor.wire;

//? if <=1.21.1 {

import net.minecraft.server.level.ServerLevel;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

/**
 * Walks a Sensor Wire network: every block connection point reachable from
 * {@code start} through wires and junctions. Used by the Wired Bus Module
 * (sensors and storage devices) and by storage blocks finding each other.
 */
public final class WireBus {

    /** Most wires one walk follows (keeps a huge or looped network cheap). */
    public static final int MAX_WIRES_WALKED = 256;

    private WireBus() {
    }

    /** Block endpoints reachable from {@code start}, nearest first (start itself excluded). */
    public static List<BlockWireEndpoint> walk(ServerLevel level, IWireEndpoint start) {
        List<BlockWireEndpoint> found = new ArrayList<>();
        Set<IWireEndpoint> seen = new HashSet<>();
        Set<BaseWireEntity> walked = new HashSet<>();
        ArrayDeque<IWireEndpoint> queue = new ArrayDeque<>();
        queue.add(start);
        seen.add(start);
        while (!queue.isEmpty() && walked.size() < MAX_WIRES_WALKED) {
            IWireEndpoint at = queue.poll();
            for (BaseWireEntity wire : WireConnections.get(level, at)) {
                if (!walked.add(wire)) continue;
                IWireEndpoint other = at.equals(wire.getEndpoint1()) ? wire.getEndpoint2() : wire.getEndpoint1();
                if (other == null || !seen.add(other)) continue;
                if (other instanceof JunctionWireEndpoint) {
                    queue.add(other);
                } else if (other instanceof BlockWireEndpoint b) {
                    found.add(b);
                }
            }
        }
        return found;
    }
}
//?}
