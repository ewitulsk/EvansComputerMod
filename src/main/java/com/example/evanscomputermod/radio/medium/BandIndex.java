package com.example.evanscomputermod.radio.medium;

import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Pose;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Spatial index of the listening radios of one band: per tuned channel, a grid
 * of {@value #CELL}-block cells (x/z, per dimension). Rebuilt lazily into an
 * immutable {@link Snapshot} when membership or a member's cell changes;
 * readers never lock once it is built. Transmit only visits the grids of
 * channels that overlap the emission and the cells inside the culling range.
 */
public final class BandIndex {

    public static final int CELL = 64;

    /** One channel's grid. */
    public static final class Grid {
        public final Channel channel;
        public final Node[] all;
        private final long[] keys;
        private final Node[][] cells;

        Grid(Channel channel, List<Node> members, Map<Node, Pose> poses) {
            this.channel = channel;
            this.all = members.toArray(new Node[0]);
            Map<Long, List<Node>> byCell = new HashMap<>();
            for (Node n : members) {
                Pose p = poses.get(n);
                byCell.computeIfAbsent(cellKey(p.dimension(), (int) Math.floor(p.x()) >> 6, (int) Math.floor(p.z()) >> 6),
                        k -> new ArrayList<>()).add(n);
            }
            int cap = 8;
            while (cap < byCell.size() * 2) cap <<= 1;
            keys = new long[cap];
            cells = new Node[cap][];
            for (Map.Entry<Long, List<Node>> e : byCell.entrySet()) {
                int mask = cap - 1;
                int i = (int) com.example.evanscomputermod.radio.phys.Fading.mix64(e.getKey()) & mask;
                while (cells[i] != null) i = (i + 1) & mask;
                keys[i] = e.getKey();
                cells[i] = e.getValue().toArray(new Node[0]);
            }
        }

        /** Members in a cell, or null. */
        public Node[] cell(long key) {
            int mask = keys.length - 1;
            for (int i = (int) com.example.evanscomputermod.radio.phys.Fading.mix64(key) & mask, n = 0; n <= mask; i = (i + 1) & mask, n++) {
                if (cells[i] == null) return null;
                if (keys[i] == key) return cells[i];
            }
            return null;
        }

        public int cellCount() {
            int n = 0;
            for (Node[] c : cells) if (c != null) n++;
            return n;
        }
    }

    /** Immutable view: grids per channel, plus the best receive gain / sensitivity for culling. */
    public static final class Snapshot {
        static final Snapshot EMPTY = new Snapshot(new Grid[0], 0, 0, 0);
        public final Grid[] grids;
        public final double maxGainDbi;
        public final double minSensitivityDbm;
        public final int size;

        Snapshot(Grid[] grids, double maxGainDbi, double minSensitivityDbm, int size) {
            this.grids = grids;
            this.maxGainDbi = maxGainDbi;
            this.minSensitivityDbm = minSensitivityDbm;
            this.size = size;
        }
    }

    private final Set<Node> members = ConcurrentHashMap.newKeySet();
    private volatile Snapshot snapshot = Snapshot.EMPTY;
    private volatile boolean dirty;

    /**
     * Cell key: dimension hash folded with cell x/z. Different dimensions may
     * collide in a cell; transmit checks the dimension anyway.
     */
    public static long cellKey(String dimension, int cx, int cz) {
        return ((long) cx << 32) ^ (cz & 0xFFFFFFFFL) ^ ((long) dimension.hashCode() * 0x9E3779B97F4A7C15L);
    }

    public void add(Node n) {
        members.add(n);
        dirty = true;
    }

    public void remove(Node n) {
        if (members.remove(n)) dirty = true;
    }

    public void markDirty() {
        dirty = true;
    }

    public int size() {
        return members.size();
    }

    public Snapshot snapshot() {
        if (dirty) {
            synchronized (this) {
                if (dirty) {
                    dirty = false;
                    snapshot = build();
                }
            }
        }
        return snapshot;
    }

    private Snapshot build() {
        Map<Channel, List<Node>> byChannel = new LinkedHashMap<>();
        Map<Node, Pose> poses = new HashMap<>();
        double maxGain = Double.NEGATIVE_INFINITY, minSens = Double.POSITIVE_INFINITY;
        int size = 0;
        for (Node n : members) {
            Channel ch = n.ep.tunedChannel();
            Pose p = n.ep.pose();
            if (ch == null || p == null) continue;
            n.indexedCellX = (int) Math.floor(p.x()) >> 6;
            n.indexedCellZ = (int) Math.floor(p.z()) >> 6;
            n.indexedChannel = ch;
            poses.put(n, p);
            byChannel.computeIfAbsent(ch, k -> new ArrayList<>()).add(n);
            maxGain = Math.max(maxGain, n.ep.antenna().peakGainDbi() - n.ep.antenna().feedLossDb());
            minSens = Math.min(minSens, n.ep.sensitivityDbm());
            size++;
        }
        Grid[] grids = new Grid[byChannel.size()];
        int i = 0;
        for (Map.Entry<Channel, List<Node>> e : byChannel.entrySet()) grids[i++] = new Grid(e.getKey(), e.getValue(), poses);
        return new Snapshot(grids, size == 0 ? 0 : maxGain, size == 0 ? 0 : minSens, size);
    }
}
