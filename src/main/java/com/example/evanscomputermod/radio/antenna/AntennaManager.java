package com.example.evanscomputermod.radio.antenna;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.radio.antenna.graph.AntennaAnalysis;
import com.example.evanscomputermod.radio.antenna.graph.AntennaGraph;
import com.example.evanscomputermod.radio.antenna.graph.AntennaReport;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.MinecraftServer;
import net.minecraft.world.level.Level;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.event.level.BlockEvent;
import net.neoforged.neoforge.event.level.LevelEvent;
import net.neoforged.neoforge.event.server.ServerStoppedEvent;

import java.util.Map;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicLong;

/**
 * The antenna cache (spec "Antenna solver", lane 5A). One entry per feed
 * point: the extracted graph, its fingerprint and the latest
 * {@link AntennaReport}. Reads never block: while a solve runs on the worker
 * thread, readers get the heuristic estimate.
 *
 * <p><b>Invalidation.</b> Every block an antenna covers (its conductors,
 * their neighbours and the column down to the ground) is indexed; a block
 * update there ({@code NeighborNotifyEvent}, or a conductor's own shape
 * update / wrench cut / weathering) marks the entry dirty. The next read
 * re-walks the graph on the main thread (cheap) and re-solves only if the
 * graph's fingerprint changed. Reads also re-walk entries older than
 * {@link #REWALK_TICKS}, which picks up Fine Wire (entity) changes.
 *
 * <p>All public methods are for the server thread, except {@link #whenSolved}'s
 * future, which completes on the server thread.
 */
public final class AntennaManager {
    public static final int REWALK_TICKS = 20;
    private static final ExecutorService WORKER = Executors.newSingleThreadExecutor(r -> {
        Thread t = new Thread(r, "ECM antenna solver");
        t.setDaemon(true);
        t.setPriority(Thread.NORM_PRIORITY - 1);
        return t;
    });
    private static final AtomicLong VERSIONS = new AtomicLong();

    private record Key(ResourceKey<Level> dim, BlockPos pos) {}

    private static final class Entry {
        final Key key;
        volatile AntennaGraph graph;
        volatile String fingerprint;
        volatile AntennaReport report;
        volatile boolean pending;
        volatile boolean dirty;
        volatile long version;
        long walkedTick;
        Set<Long> watched = Set.of();
        CompletableFuture<Antenna> solved = new CompletableFuture<>();

        Entry(Key key) { this.key = key; }

        Antenna snapshot() { return new Antenna(key.dim, key.pos, report, graph, pending, version); }
    }

    private static final Map<Key, Entry> CACHE = new ConcurrentHashMap<>();
    /** dimension → block pos (long) → feed points watching it. */
    private static final Map<ResourceKey<Level>, Map<Long, Set<BlockPos>>> WATCH = new ConcurrentHashMap<>();

    private AntennaManager() {}

    public static void register() {
        NeoForge.EVENT_BUS.addListener((BlockEvent.NeighborNotifyEvent e) -> {
            if (e.getLevel() instanceof Level l && !l.isClientSide()) blockChanged(l, e.getPos());
        });
        NeoForge.EVENT_BUS.addListener((LevelEvent.Unload e) -> {
            if (e.getLevel() instanceof Level l) {
                CACHE.keySet().removeIf(k -> k.dim == l.dimension());
                WATCH.remove(l.dimension());
            }
        });
        NeoForge.EVENT_BUS.addListener((ServerStoppedEvent e) -> {
            CACHE.clear();
            WATCH.clear();
        });
    }

    // ------------------------------------------------------------ reads

    /** The antenna on the feed point at {@code feed} (an estimate while solving); "no antenna" if there is no feed point. */
    public static Antenna get(Level level, BlockPos feed) {
        Entry e = refresh(level, feed.immutable());
        if (e == null) return new Antenna(level.dimension(), feed.immutable(), AntennaReport.none("no feed point here"), null, false, 0);
        return e.snapshot();
    }

    /** Completes (on the server thread) with the solved antenna; completes at once if already solved. */
    public static CompletableFuture<Antenna> whenSolved(Level level, BlockPos feed) {
        Entry e = refresh(level, feed.immutable());
        if (e == null) return CompletableFuture.completedFuture(get(level, feed));
        if (!e.pending) return CompletableFuture.completedFuture(e.snapshot());
        return e.solved;
    }

    /**
     * Solves now, blocking the calling (server) thread up to {@code timeoutMs}
     * for the worker. For commands, scenarios and tests; gameplay code uses
     * {@link #get} / {@link #whenSolved}.
     */
    public static Antenna solveNow(Level level, BlockPos feed, long timeoutMs) {
        Entry e = refresh(level, feed.immutable());
        if (e == null) return get(level, feed);
        long deadline = System.currentTimeMillis() + timeoutMs;
        while (e.pending && System.currentTimeMillis() < deadline) {
            try {
                Thread.sleep(2);
            } catch (InterruptedException ie) {
                Thread.currentThread().interrupt();
                break;
            }
        }
        return e.snapshot();
    }

    // ------------------------------------------------------------ invalidation

    /** A block at {@code pos} changed: dirty every antenna watching it. */
    public static void blockChanged(Level level, BlockPos pos) {
        Map<Long, Set<BlockPos>> idx = WATCH.get(level.dimension());
        if (idx == null) return;
        Set<BlockPos> feeds = idx.get(pos.asLong());
        if (feeds == null) return;
        for (BlockPos f : feeds) {
            Entry e = CACHE.get(new Key(level.dimension(), f));
            if (e != null) e.dirty = true;
        }
    }

    /** The feed point at {@code feed} is gone. */
    public static void forget(Level level, BlockPos feed) {
        Entry e = CACHE.remove(new Key(level.dimension(), feed.immutable()));
        if (e != null) unwatch(level.dimension(), e);
    }

    // ------------------------------------------------------------ internals

    private static Entry refresh(Level level, BlockPos feed) {
        Key key = new Key(level.dimension(), feed);
        Entry e = CACHE.get(key);
        long now = level.getGameTime();
        if (e != null && !e.dirty && now - e.walkedTick < REWALK_TICKS) return e;
        AntennaGraphWalker.Walk walk = AntennaGraphWalker.walk(level, feed);
        if (walk == null) {
            if (e != null) forget(level, feed);
            return null;
        }
        String fp = walk.graph().fingerprint();
        if (e != null && fp.equals(e.fingerprint)) {
            e.dirty = false;
            e.walkedTick = now;
            return e;
        }
        if (e == null) {
            e = new Entry(key);
            CACHE.put(key, e);
        } else {
            unwatch(key.dim, e);
        }
        e.graph = walk.graph();
        e.fingerprint = fp;
        e.dirty = false;
        e.walkedTick = now;
        e.watched = watchSet(walk, feed, walk.graph());
        watch(key.dim, e);
        submit(level.getServer(), e);
        return e;
    }

    private static void submit(MinecraftServer server, Entry e) {
        AntennaGraph g = e.graph;
        long version = VERSIONS.incrementAndGet();
        e.version = version;
        if (g.empty()) {
            e.report = AntennaAnalysis.solve(g);
            e.pending = false;
            complete(e);
            return;
        }
        e.report = AntennaAnalysis.estimate(g);
        e.pending = true;
        if (e.solved.isDone()) e.solved = new CompletableFuture<>();
        WORKER.execute(() -> {
            AntennaReport r;
            try {
                r = AntennaAnalysis.solve(g);
            } catch (RuntimeException ex) {
                EvansComputerMod.LOGGER.warn("Antenna solve at {} failed", e.key.pos, ex);
                r = AntennaReport.invalid("solver error: " + ex, g.totalLength(), g.groundName);
            }
            AntennaReport result = r;
            if (e.version != version) return;   // superseded by a newer graph
            e.report = result;
            e.pending = false;
            if (server != null) server.execute(() -> complete(e));
            else complete(e);
        });
    }

    private static void complete(Entry e) {
        e.solved.complete(e.snapshot());
    }

    private static Set<Long> watchSet(AntennaGraphWalker.Walk walk, BlockPos feed, AntennaGraph g) {
        Set<Long> s = new java.util.HashSet<>();
        for (BlockPos p : walk.blocks()) {
            s.add(p.asLong());
            for (Direction d : Direction.values()) s.add(p.relative(d).asLong());
        }
        int bottom = Double.isNaN(g.groundY) ? feed.getY() - 1 : (int) Math.floor(g.groundY) - 1;
        for (int y = feed.getY() - 1; y >= bottom && feed.getY() - y <= 384; y--) s.add(new BlockPos(feed.getX(), y, feed.getZ()).asLong());
        return s;
    }

    private static void watch(ResourceKey<Level> dim, Entry e) {
        Map<Long, Set<BlockPos>> idx = WATCH.computeIfAbsent(dim, d -> new ConcurrentHashMap<>());
        for (long p : e.watched) idx.computeIfAbsent(p, x -> ConcurrentHashMap.newKeySet()).add(e.key.pos);
    }

    private static void unwatch(ResourceKey<Level> dim, Entry e) {
        Map<Long, Set<BlockPos>> idx = WATCH.get(dim);
        if (idx == null) return;
        for (long p : e.watched) {
            Set<BlockPos> s = idx.get(p);
            if (s != null) {
                s.remove(e.key.pos);
                if (s.isEmpty()) idx.remove(p);
            }
        }
    }
}
//?}
