package com.example.evanscomputermod.radio.medium;

import com.example.evanscomputermod.radio.RadioConfig;
import com.example.evanscomputermod.radio.api.Band;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;
import com.example.evanscomputermod.radio.phys.Fading;
import com.example.evanscomputermod.radio.phys.FreeSpace;
import com.example.evanscomputermod.radio.phys.Ionosphere;
import com.example.evanscomputermod.radio.phys.Noise;
import com.example.evanscomputermod.radio.phys.PathLossModel;
import com.example.evanscomputermod.radio.phys.Polarization;
import com.example.evanscomputermod.radio.phys.PolarizationLoss;
import com.example.evanscomputermod.radio.phys.SpectralMask;
import com.example.evanscomputermod.radio.phys.Units;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;
import java.util.concurrent.atomic.AtomicLongArray;
import java.util.function.LongSupplier;

/**
 * The in-world {@link RadioMedium} (lane 3A/5B): every pair's path loss comes
 * from the voxel/heightmap {@link PathTracer} and is cached in a {@link LinkCache}
 * snapshot. {@link #transmit} never touches the world: it reads the cache
 * (free-space distance delta from the traced geometry plus the cached excess),
 * adds fading, interference through spectral masks and noise, and rolls once
 * against the packet error rate.
 *
 * <p><b>Threads.</b> {@code transmit}, {@code channelPowerDbm}, {@code pathGainDb},
 * {@code register}, {@code unregister} and {@code invalidate} are thread-safe and
 * lock-free on the hot path. {@link #tick} (server thread) owns all world access:
 * it notices moved/turned/retuned endpoints, drains link requests, recomputes
 * pairs within the per-tick ray budget, and publishes the cache.
 *
 * <p><b>Sharding.</b> Emissions live in one {@link Airwaves} ring per band and
 * receivers in one {@link BandIndex} per band (a grid per tuned channel); bands
 * share no mutable state, so different bands never contend.
 *
 * <p><b>Unknown pairs</b> (not traced yet) use free space plus
 * {@link #UNKNOWN_MARGIN_DB} and are queued for tracing at once.
 */
public class WorldRadioMedium implements RadioMedium {

    /** Pessimistic clutter allowance for a pair that hasn't been traced yet, dB. */
    public static final double UNKNOWN_MARGIN_DB = 10;
    /** Endpoints discover listeners within this many blocks on registration/moves. */
    public static final double DISCOVER_RADIUS = 128;
    /** At most this many neighbours are queued per discovery (the rest are traced when they first talk). */
    public static final int DISCOVER_MAX = 64;
    /**
     * Pairs that could use the ionosphere (traced below 30 MHz under a sky) are re-traced this
     * often while the dimension's time of day moves: day/night moves the MUF and D-layer
     * absorption, so a ground-wave pair can switch into skywave and back.
     */
    public static final int SKY_REFRESH_TICKS = 600;
    /** Trace frequency bins per octave: one cached trace serves frequencies within a quarter octave (±9%). */
    public static final int BINS_PER_OCTAVE = 4;
    private static final double BIN_BASE_HZ = 1000;
    private static final double SPEED_OF_LIGHT = 299_792_458.0;
    private static final int SHARDS = Band.values().length + 1;
    private static final int WANT_SIZE = 1 << 16;

    /** What the medium needs from the game, read only by {@link #tick} (server thread). */
    public interface WorldAccess {
        /** Blocks of a dimension, or null if it isn't loaded. */
        RfWorld world(String dimension);

        /** Weather and time in a dimension (null = defaults). */
        Conditions conditions(String dimension);

        /** Identity of the Sable sub-level a world position is on (for per-ship rate limits), or null. */
        default Object subLevelOf(String dimension, double x, double y, double z) {
            return null;
        }
    }

    /** Per-dimension conditions read by the hot path. */
    public record Conditions(long dayTime, boolean thundering, boolean hasSky) {
        public static final Conditions DEFAULT = new Conditions(6000, false, true);
    }

    private final LongSupplier clock;
    private final long seed;
    private final Map<UUID, Node> nodes = new ConcurrentHashMap<>();
    private final AtomicInteger nextIdx = new AtomicInteger(1);
    private final Airwaves[] air = new Airwaves[SHARDS];
    private final BandIndex[] index = new BandIndex[SHARDS];
    private final LinkCache cache = new LinkCache();
    private final Map<String, Conditions> conditions = new ConcurrentHashMap<>();
    private final ConcurrentLinkedQueue<Node> added = new ConcurrentLinkedQueue<>();
    private final ConcurrentLinkedQueue<Node> removedQ = new ConcurrentLinkedQueue<>();
    private final Object indexLock = new Object();

    // Link requests from any thread: lock-free ring + a small "already asked" filter.
    private final AtomicLongArray wanted = new AtomicLongArray(WANT_SIZE);
    private final AtomicLong wantHead = new AtomicLong();
    private long wantTail;
    private final AtomicLongArray asked = new AtomicLongArray(4096);

    // ---- server-thread state
    private final Map<Integer, Node> byIdx = new HashMap<>();
    private final ArrayDeque<Long> urgent = new ArrayDeque<>();
    private final ArrayDeque<Long> background = new ArrayDeque<>();
    private final Set<Long> queued = new HashSet<>();
    private final Map<Integer, Set<Long>> pairsOf = new HashMap<>();
    private final Map<Long, PairDeps> pairDeps = new HashMap<>();
    private final Map<String, Map<Long, Set<Long>>> depPairs = new HashMap<>();
    private final Set<Long> skyPairs = new HashSet<>();
    /** Pairs traced below 30 MHz under a sky (skywave possible) → their dimension. */
    private final Map<Long, String> ionoPairs = new HashMap<>();
    /** Day time of each dimension when its ionosphere pairs were last re-queued. */
    private final Map<String, Long> ionoDayTime = new HashMap<>();
    private final Map<Object, Long> subLevelLastTick = new HashMap<>();
    private final PathTracer tracer = new PathTracer();
    private final RayBudget budget;
    private long currentTick;
    private int computedLastTick;
    private long computedTotal;
    private volatile double kLosLinear = Fading.kFromDb(Fading.DEFAULT_LOS_K_DB);

    private record PairDeps(String dimension, long[] deps) {}

    public WorldRadioMedium(LongSupplier clockMicros, long seed) {
        this.clock = clockMicros;
        this.seed = seed;
        this.budget = new RayBudget(RadioConfig.raysPerTick());
        for (int i = 0; i < SHARDS; i++) {
            air[i] = new Airwaves();
            index[i] = new BandIndex();
        }
    }

    // ------------------------------------------------------------------ RadioMedium

    @Override
    public long nowMicros() {
        return clock.getAsLong();
    }

    @Override
    public void register(RadioEndpoint endpoint) {
        Node old = nodes.get(endpoint.id());
        if (old != null && old.ep == endpoint && !old.removed) return;
        Node n = new Node(endpoint, nextIdx.getAndIncrement());
        if (n.idx >= (1 << 28)) throw new IllegalStateException("radio endpoint index space exhausted");
        Node prev = nodes.put(endpoint.id(), n);
        if (prev != null) retire(prev);
        reindex(n);
        added.add(n);
    }

    @Override
    public void unregister(RadioEndpoint endpoint) {
        Node n = nodes.get(endpoint.id());
        if (n != null && nodes.remove(endpoint.id(), n)) retire(n);
    }

    private void retire(Node n) {
        n.removed = true;
        synchronized (indexLock) {
            if (n.indexBand >= 0) index[n.indexBand].remove(n);
            n.indexBand = -1;
        }
        removedQ.add(n);
    }

    @Override
    public void invalidate(RadioEndpoint endpoint) {
        Node n = nodes.get(endpoint.id());
        if (n == null) return;
        n.invalidated = true;
        reindex(n);
    }

    /** Put a node in the index of its tuned band (or none). */
    private void reindex(Node n) {
        Channel ch = n.ep.tunedChannel();
        int band = ch == null ? -1 : shard(ch.centerHz());
        synchronized (indexLock) {
            if (n.removed) return;
            if (band != n.indexBand) {
                if (n.indexBand >= 0) index[n.indexBand].remove(n);
                if (band >= 0) index[band].add(n);
                n.indexBand = band;
            } else if (band >= 0) {
                index[band].markDirty();
            }
        }
    }

    @Override
    public Emission transmit(RadioEndpoint from, Emission e) {
        long now = nowMicros();
        if (e.startMicros() < now) {
            e = new Emission(e.kind(), e.channel(), e.powerDbm(), now, e.durationMicros(), e.modulation(),
                    e.bitRate(), e.payload(), e.iq(), e.sampleRateHz());
        }
        Node tx = nodes.get(from.id());
        if (tx == null || tx.removed) {
            register(from);
            tx = nodes.get(from.id());
        }
        Channel ch = e.channel();
        double fc = ch.centerHz();
        int shard = shard(fc);
        tx.lastTxHz = fc;
        tx.lastTxBand = shard;
        Pose txPose = from.pose();
        PerModels.Model model = PerModels.of(e.modulation());
        Airwaves.Active self = new Airwaves.Active(e, tx, txPose, model.mask());
        Airwaves waves = air[shard];
        waves.add(self);
        if (e.kind() != Emission.Kind.FRAME) return e;

        BandIndex.Snapshot snap = index[shard].snapshot();
        if (snap.size == 0) return e;
        double txGain = from.antenna().peakGainDbi() - from.antenna().feedLossDb();
        double range = FreeSpace.distanceForLossM(e.powerDbm() + txGain + snap.maxGainDbi - snap.minSensitivityDbm + 6, fc);
        int bits = e.payload() == null ? 0 : e.payload().length * 8;
        int r = (int) Math.min(1 << 20, Math.ceil(range / BandIndex.CELL));
        int cx = (int) Math.floor(txPose.x()) >> 6, cz = (int) Math.floor(txPose.z()) >> 6;
        long span = (2L * r + 1) * (2L * r + 1);
        for (BandIndex.Grid grid : snap.grids) {
            if (!grid.channel.overlaps(ch)) continue;
            if (span >= grid.all.length || span > 4096) {
                for (Node rx : grid.all) deliver(tx, from, txPose, e, self, waves, rx, range, bits, model, now);
            } else {
                for (int x = cx - r; x <= cx + r; x++)
                    for (int z = cz - r; z <= cz + r; z++) {
                        Node[] cell = grid.cell(BandIndex.cellKey(txPose.dimension(), x, z));
                        if (cell == null) continue;
                        for (Node rx : cell) deliver(tx, from, txPose, e, self, waves, rx, range, bits, model, now);
                    }
            }
        }
        return e;
    }

    private void deliver(Node tx, RadioEndpoint from, Pose txPose, Emission e, Airwaves.Active self, Airwaves waves,
                         Node rxNode, double range, int bits, PerModels.Model model, long now) {
        if (rxNode == tx || rxNode.removed) return;
        RadioEndpoint rx = rxNode.ep;
        Pose rxPose = rx.pose();
        if (!rxPose.sameDimension(txPose)) return;
        double d = distance(txPose, rxPose);
        if (d > range || !rx.listening()) return;
        Channel tuned = rx.tunedChannel();
        if (tuned == null || !tuned.overlaps(e.channel())) return;
        double w = self.mask().weight(e.channel().centerHz(), e.channel().bandwidthHz(), tuned.centerHz(), tuned.bandwidthHz());
        if (w <= 0) return;
        double rssi = e.powerDbm() + linkDb(tx, txPose, rxNode, rxPose, d, e.channel().centerHz(), now, true)
                + 10 * Math.log10(w);
        if (rssi < rx.sensitivityDbm()) return;
        Conditions c = conditionsOf(rxPose.dimension());
        double noiseDbm = Noise.floorDbm(tuned.bandwidthHz(), rx.noiseFigureDb(), tuned.centerHz(),
                Noise.Environment.RURAL, c.dayTime(), c.thundering());
        double interfMw = interferenceMw(waves, self, e.startMicros(), e.endMicros(), rxNode, rxPose, tuned, now);
        double sinr = rssi - Units.mwToDbm(Units.dbmToMw(noiseDbm) + interfMw);
        double per = model.per(sinr, bits, tuned.bandwidthHz(), e.bitRate());
        if (roll(e, rx.id()) < per) return;
        long delay = Math.round(d / SPEED_OF_LIGHT * 1e6);
        rx.onReceive(new Reception(from.id(), e, rssi, sinr, e.endMicros() + delay));
    }

    /** Sum of every other emission overlapping [start, end) heard at {@code rx} in its passband, mW. */
    private double interferenceMw(Airwaves waves, Airwaves.Active self, long start, long end, Node rx, Pose rxPose,
                                  Channel tuned, long now) {
        double mw = 0;
        long h = waves.head();
        for (long i = h - 1; i >= Airwaves.tail(h); i--) {
            Airwaves.Active a = waves.at(i);
            if (a == null) continue;
            if (waves.olderThan(a, start)) break;
            mw += interferenceOf(a, self, start, end, rx, rxPose, tuned, now);
        }
        for (Airwaves.Active a : waves.longs()) mw += interferenceOf(a, self, start, end, rx, rxPose, tuned, now);
        return mw;
    }

    private double interferenceOf(Airwaves.Active a, Airwaves.Active self, long start, long end, Node rx, Pose rxPose,
                                  Channel tuned, long now) {
        Emission o = a.emission();
        if (a == self || a.sender() == rx || o.startMicros() >= end || o.endMicros() <= start) return 0;
        if (!a.pose().sameDimension(rxPose)) return 0;
        double w = a.mask().weight(o.channel().centerHz(), o.channel().bandwidthHz(), tuned.centerHz(), tuned.bandwidthHz());
        if (w < 1e-9) return 0;
        double p = o.powerDbm() + linkDb(a.sender(), a.pose(), rx, rxPose, distance(a.pose(), rxPose),
                o.channel().centerHz(), now, true);
        return w * Units.dbmToMw(p);
    }

    @Override
    public double channelPowerDbm(RadioEndpoint at, Channel channel) {
        long now = nowMicros();
        Node rx = nodes.get(at.id());
        Pose rxPose = at.pose();
        Airwaves waves = air[shard(channel.centerHz())];
        double mw = 0;
        long h = waves.head();
        for (long i = h - 1; i >= Airwaves.tail(h); i--) {
            Airwaves.Active a = waves.at(i);
            if (a == null) continue;
            if (waves.olderThan(a, now)) break;
            mw += powerOf(a, at, rx, rxPose, channel, now);
        }
        for (Airwaves.Active a : waves.longs()) mw += powerOf(a, at, rx, rxPose, channel, now);
        return mw <= 0 ? Double.NEGATIVE_INFINITY : Units.mwToDbm(mw);
    }

    private double powerOf(Airwaves.Active a, RadioEndpoint at, Node rx, Pose rxPose, Channel channel, long now) {
        Emission o = a.emission();
        if (a.sender().id.equals(at.id()) || o.startMicros() > now || o.endMicros() < now) return 0;
        if (!a.pose().sameDimension(rxPose)) return 0;
        double w = a.mask().weight(o.channel().centerHz(), o.channel().bandwidthHz(), channel.centerHz(), channel.bandwidthHz());
        if (w < 1e-9) return 0;
        double d = distance(a.pose(), rxPose);
        double p = rx == null
                ? o.powerDbm() - FreeSpace.lossDb(Math.max(1, d), o.channel().centerHz()) - UNKNOWN_MARGIN_DB
                : o.powerDbm() + linkDb(a.sender(), a.pose(), rx, rxPose, d, o.channel().centerHz(), now, true);
        return w * Units.dbmToMw(p);
    }

    /**
     * Every emission overlapping the window and channel as heard at {@code rx}: power at the
     * antenna port from the cached path (antenna gains, polarization, feed losses, no fading -
     * SDR synthesis applies its own fading phasor) and the propagation delay. Reads the
     * band shard's ring only; no world access.
     */
    @Override
    public void forEachHeard(RadioEndpoint rx, Channel within, long fromMicros, long toMicros,
                             java.util.function.Consumer<Heard> sink) {
        Pose rxPose = rx.pose();
        if (rxPose == null) return;
        Node rn = nodes.get(rx.id());
        long now = nowMicros();
        Airwaves waves = air[shard(within.centerHz())];
        long h = waves.head();
        for (long i = h - 1; i >= Airwaves.tail(h); i--) {
            Airwaves.Active a = waves.at(i);
            if (a == null) continue;
            if (waves.olderThan(a, fromMicros)) break;
            heard(a, rx, rn, rxPose, within, fromMicros, toMicros, now, sink);
        }
        for (Airwaves.Active a : waves.longs()) heard(a, rx, rn, rxPose, within, fromMicros, toMicros, now, sink);
    }

    private void heard(Airwaves.Active a, RadioEndpoint rx, Node rn, Pose rxPose, Channel within, long fromMicros,
                       long toMicros, long now, java.util.function.Consumer<Heard> sink) {
        Emission e = a.emission();
        if (a.sender().id.equals(rx.id()) || e.endMicros() <= fromMicros || e.startMicros() >= toMicros) return;
        if (!e.channel().overlaps(within) || !a.pose().sameDimension(rxPose)) return;
        double d = distance(a.pose(), rxPose);
        double f = e.channel().centerHz();
        double p = rn == null
                ? e.powerDbm() + a.sender().ep.antenna().peakGainDbi() + rx.antenna().peakGainDbi()
                        - FreeSpace.lossDb(Math.max(1, d), f) - UNKNOWN_MARGIN_DB
                : e.powerDbm() + linkDb(a.sender(), a.pose(), rn, rxPose, d, f, now, false);
        sink.accept(new Heard(a.sender().ep, e, p, d / SPEED_OF_LIGHT * 1e6));
    }

    @Override
    public double pathGainDb(RadioEndpoint a, RadioEndpoint b, double freqHz) {
        Pose pa = a.pose(), pb = b.pose();
        if (!pa.sameDimension(pb)) return Double.NEGATIVE_INFINITY;
        Node na = nodes.get(a.id()), nb = nodes.get(b.id());
        if (na == null || nb == null) return Double.NaN;
        long key = pairKey(na.idx, nb.idx, traceBin(freqHz));
        LinkCache.Link l = cache.get(key);
        if (l == null) {
            request(key);
            return Double.NaN;
        }
        return -Math.max(0, FreeSpace.lossDb(Math.max(1, distance(pa, pb)), freqHz) + l.excessDb());
    }

    /** Cached link details for two endpoints at a frequency (debug, tests), or null if not traced yet. */
    public LinkCache.Link link(RadioEndpoint a, RadioEndpoint b, double freqHz) {
        Node na = nodes.get(a.id()), nb = nodes.get(b.id());
        if (na == null || nb == null) return null;
        long key = pairKey(na.idx, nb.idx, traceBin(freqHz));
        LinkCache.Link l = cache.get(key);
        if (l == null) request(key);
        return l;
    }

    /**
     * Net gain tx→rx excluding transmit power: antenna gains − feed losses −
     * path loss − polarization (+ fading). Reads only the cache.
     */
    private double linkDb(Node t, Pose tp, Node r, Pose rp, double d, double f, long now, boolean fade) {
        long key = pairKey(t.idx, r.idx, traceBin(f));
        LinkCache.Link l = cache.get(key);
        double g, loss, k;
        if (l != null) {
            boolean tIsA = t.idx < r.idx;
            g = (tIsA ? l.gainA() + l.gainB() : l.gainB() + l.gainA()) - l.polDb();
            // Path loss is never a gain (the cached excess can be negative where ground reflection adds up).
            loss = Math.max(0, FreeSpace.lossDb(Math.max(1, d), f) + l.excessDb());
            k = l.kLinear();
        } else {
            request(key);
            g = t.ep.antenna().peakGainDbi() + r.ep.antenna().peakGainDbi();
            loss = FreeSpace.lossDb(Math.max(1, d), f) + UNKNOWN_MARGIN_DB;
            k = kLosLinear;
        }
        g -= t.ep.antenna().feedLossDb() + r.ep.antenna().feedLossDb();
        if (fade) {
            double coh = Fading.coherenceTimeS(t.ep.speedMps() + r.ep.speedMps(), f);
            long s = Fading.blockSeed(seed ^ key, now * 1e-6, coh);
            g += k > 0 ? Fading.ricianGainDb(k, s) : Fading.rayleighGainDb(s);
        }
        return g - loss;
    }

    private double roll(Emission e, UUID rx) {
        long h = seed ^ e.startMicros() * 0x9E3779B97F4A7C15L ^ rx.getMostSignificantBits() ^ rx.getLeastSignificantBits() * 31;
        if (e.payload() != null) h ^= java.util.Arrays.hashCode(e.payload());
        return (Fading.mix64(h) >>> 11) * 0x1.0p-53;
    }

    private Conditions conditionsOf(String dim) {
        Conditions c = conditions.get(dim);
        return c == null ? Conditions.DEFAULT : c;
    }

    // ------------------------------------------------------------------ keys and requests

    /**
     * Unordered pair + trace frequency bin: lo(28 bits) | hi(28 bits) | bin(8 bits). Never 0
     * (endpoint indices start at 1). One trace is cached per bin, so frequency-dependent loss
     * (walls, diffraction, ground, skywave) is only shared by frequencies within a quarter octave.
     */
    public static long pairKey(int i, int j, int bin) {
        int lo = Math.min(i, j), hi = Math.max(i, j);
        return ((long) lo << 36) | ((long) hi << 8) | (bin & 0xFF);
    }

    static int keyLo(long key) {
        return (int) (key >>> 36);
    }

    static int keyHi(long key) {
        return (int) ((key >>> 8) & ((1 << 28) - 1));
    }

    static int keyBin(long key) {
        return (int) (key & 0xFF);
    }

    /** Quarter-octave trace bin of a frequency (1..255, from 1 kHz up); 0 for none. */
    public static int traceBin(double hz) {
        if (!(hz > 0)) return 0;
        double b = 1 + Math.floor(BINS_PER_OCTAVE * Math.log(hz / BIN_BASE_HZ) / Math.log(2));
        return (int) Math.max(1, Math.min(255, b));
    }

    /** Geometric centre frequency of a trace bin, Hz. */
    static double binCenterHz(int bin) {
        return BIN_BASE_HZ * Math.pow(2, (bin - 0.5) / BINS_PER_OCTAVE);
    }

    /** The band shard of a frequency (the last shard holds frequencies outside every band). */
    public static int shard(double hz) {
        Band b = Band.of(hz);
        return b == null ? SHARDS - 1 : b.ordinal();
    }

    void request(long key) {
        int slot = (int) (Fading.mix64(key) & (asked.length() - 1));
        if (asked.get(slot) == key) return;
        asked.set(slot, key);
        long i = wantHead.getAndIncrement();
        wanted.set((int) (i & (WANT_SIZE - 1)), key);
    }

    // ------------------------------------------------------------------ server tick

    /**
     * Server-thread maintenance: retire removed endpoints, detect moves/turns/retunes
     * (rate-limited per Sable sub-level), drain requests, recompute pairs within the
     * ray budget, publish the cache.
     */
    public void tick(long gameTick, WorldAccess access) {
        currentTick = gameTick;
        kLosLinear = switch (RadioConfig.realism()) {
            case ARCADE -> Fading.kFromDb(15);
            case REALISTIC -> Fading.kFromDb(Fading.DEFAULT_LOS_K_DB);
            case SIMULATION -> Fading.kFromDb(6);
        };
        for (Node n; (n = removedQ.poll()) != null; ) dropNode(n);
        for (Node n; (n = added.poll()) != null; ) if (!n.removed) byIdx.put(n.idx, n);

        // One movement policy for every endpoint (hardware never invalidates on movement): the medium
        // notices moves/turns past the configured thresholds itself. Antenna terms (gain towards the
        // other end, polarization) follow at once since they cost no rays; path retraces are held to
        // one round per minRecomputeTicks per Sable sub-level (per endpoint off ships); in between
        // the cached path is interpolated with the free-space distance delta.
        double moveM = RadioConfig.sableRecomputeMetres(), turnRad = RadioConfig.sableRecomputeRadians();
        int minTicks = RadioConfig.sableMinRecomputeTicks();
        Set<String> dims = new HashSet<>();
        for (Node n : byIdx.values()) {
            Pose p = n.ep.pose();
            if (p == null) continue;
            dims.add(p.dimension());
            if (n.invalidated) {
                // An explicit change of the antenna or channel (not movement): gains at once, paths now.
                n.invalidated = false;
                refreshGains(n);
                requeueAll(n, true);
                n.computedPose = p;
                n.gainPose = p;
                n.movePending = false;
                n.discover = true;
            } else if (n.computedPose == null) {
                n.computedPose = p;
                n.gainPose = p;
            } else {
                double turn = turnRad * Math.max(1e-3, n.ep.turnThresholdScale());
                if (n.gainPose == null || p.movedBeyond(n.gainPose, moveM, turn)) {
                    n.gainPose = p;
                    refreshGains(n);
                }
                if (n.movePending || p.movedBeyond(n.computedPose, moveM, turnRad)) {
                    Object limiter = access == null ? null : access.subLevelOf(p.dimension(), p.x(), p.y(), p.z());
                    Long last = limiter == null ? n.lastMoveTick : subLevelLastTick.getOrDefault(limiter, Long.MIN_VALUE / 2);
                    if (gameTick - last >= minTicks) {
                        if (limiter != null) subLevelLastTick.put(limiter, gameTick);
                        n.lastMoveTick = gameTick;
                        n.movePending = false;
                        n.computedPose = p;
                        requeueAll(n, false);
                        n.discover = true;
                    } else {
                        n.movePending = true;   // interpolate (free-space delta) until the limiter allows
                    }
                }
            }
            Channel ch = n.ep.tunedChannel();
            int band = ch == null ? -1 : shard(ch.centerHz());
            if (band != n.indexBand || ch != null && !ch.equals(n.indexedChannel)
                    || band >= 0 && ((int) Math.floor(p.x()) >> 6 != n.indexedCellX || (int) Math.floor(p.z()) >> 6 != n.indexedCellZ))
                reindex(n);
            if (n.discover) {
                n.discover = false;
                discover(n, p);
            }
        }
        if (access != null)
            for (String d : dims) {
                Conditions c = access.conditions(d);
                conditions.put(d, c == null ? Conditions.DEFAULT : c);
            }
        if (gameTick % SKY_REFRESH_TICKS == 0) refreshIonosphere();
        drainRequests();
        computedLastTick = 0;
        if (access != null) recompute(access, gameTick);
        cache.publish();
    }

    /**
     * Re-queue (background, within the ray budget) every pair that could use the ionosphere in a
     * dimension whose time of day moved since the last refresh: pairs on ground wave can switch
     * into skywave as much as skywave pairs can drop out. A dimension with a frozen clock costs nothing.
     */
    private void refreshIonosphere() {
        Map<String, Boolean> moved = new HashMap<>();
        for (Map.Entry<Long, String> e : ionoPairs.entrySet()) {
            boolean m = moved.computeIfAbsent(e.getValue(), d -> {
                long now = conditionsOf(d).dayTime();
                Long before = ionoDayTime.put(d, now);
                return before == null || before != now;
            });
            if (m) enqueue(e.getKey(), false);
        }
        for (Long k : skyPairs) enqueue(k, false);
    }

    /** How many pairs are watched for ionosphere changes (tests, debug). */
    public int ionospherePairs() {
        return ionoPairs.size();
    }

    private void discover(Node n, Pose p) {
        Channel ch = n.ep.tunedChannel();
        double hz = ch != null ? ch.centerHz() : n.lastTxHz;
        int band = ch != null ? shard(ch.centerHz()) : n.lastTxBand;
        if (band < 0 || !(hz > 0)) return;
        int bin = traceBin(hz);
        BandIndex.Snapshot snap = index[band].snapshot();
        int r = (int) Math.ceil(DISCOVER_RADIUS / BandIndex.CELL), found = 0;
        int cx = (int) Math.floor(p.x()) >> 6, cz = (int) Math.floor(p.z()) >> 6;
        for (BandIndex.Grid g : snap.grids)
            for (int x = cx - r; x <= cx + r; x++)
                for (int z = cz - r; z <= cz + r; z++) {
                    Node[] cell = g.cell(BandIndex.cellKey(p.dimension(), x, z));
                    if (cell == null) continue;
                    for (Node o : cell) {
                        if (o == n || o.removed) continue;
                        Pose op = o.ep.pose();
                        if (op != null && op.sameDimension(p) && distance(op, p) <= DISCOVER_RADIUS) {
                            enqueue(pairKey(n.idx, o.idx, bin), true);
                            if (++found >= DISCOVER_MAX) return;
                        }
                    }
                }
    }

    private void drainRequests() {
        long h = wantHead.get();
        if (h - wantTail > WANT_SIZE) {
            // The ring overflowed: the oldest requests were overwritten and are lost. Their
            // "already asked" slots must not stay set, or those pairs could never ask again.
            wantTail = h - WANT_SIZE;
            for (int i = 0; i < asked.length(); i++) asked.set(i, 0);
        }
        for (long i = wantTail; i < h; i++) {
            long k = wanted.getAndSet((int) (i & (WANT_SIZE - 1)), 0);
            if (k == 0) continue;
            int slot = (int) (Fading.mix64(k) & (asked.length() - 1));
            asked.compareAndSet(slot, k, 0);
            enqueue(k, true);
        }
        wantTail = h;
    }

    private void enqueue(long key, boolean isUrgent) {
        if (!queued.add(key)) return;
        (isUrgent ? urgent : background).add(key);
    }

    /**
     * Recompute only the antenna terms (gains, polarization) of every cached link
     * of {@code n}, keeping the traced path loss: an antenna that turned or retuned
     * (a dish re-aimed, a tuner changed) takes effect on the next tick instead of
     * waiting for the path retrace in the ray budget.
     */
    private void refreshGains(Node n) {
        Set<Long> s = pairsOf.get(n.idx);
        if (s == null) return;
        for (Long k : s) {
            LinkCache.Link old = cache.get(k);
            if (old == null) continue;
            Node lo = byIdx.get(keyLo(k)), hi = byIdx.get(keyHi(k));
            if (lo == null || hi == null) continue;
            Pose pa = lo.ep.pose(), pb = hi.ep.pose();
            if (pa == null || pb == null) continue;
            double dx = pb.x() - pa.x(), dy = pb.y() - pa.y(), dz = pb.z() - pa.z();
            double d = Math.max(1e-6, Math.sqrt(dx * dx + dy * dy + dz * dz));
            double ux = dx / d, uy = dy / d, uz = dz / d;
            double[] la = pa.toLocal(ux, uy, uz), lb = pb.toLocal(-ux, -uy, -uz);
            double gLo = lo.ep.antenna().gainDbi(la[0], la[1], la[2]);
            double gHi = hi.ep.antenna().gainDbi(lb[0], lb[1], lb[2]);
            double pol = old.polDb();
            if (!skyPairs.contains(k)) {
                double[] ea = worldPol(pa, lo.ep.antenna().polarization(la[0], la[1], la[2]));
                double[] eb = worldPol(pb, hi.ep.antenna().polarization(lb[0], lb[1], lb[2]));
                pol = polarizationLossDb(ea, eb, ux, uy, uz);
            }
            cache.put(k, new LinkCache.Link(old.excessDb(), old.freqHz(), gLo, gHi, pol, old.kLinear(),
                    old.lineOfSight(), old.computedTick(), old.path()));
        }
    }

    private void requeueAll(Node n, boolean isUrgent) {
        Set<Long> s = pairsOf.get(n.idx);
        if (s != null) for (Long k : s) enqueue(k, isUrgent);
    }

    private void dropNode(Node n) {
        byIdx.remove(n.idx);
        Set<Long> s = pairsOf.remove(n.idx);
        if (s == null) return;
        for (Long k : s) {
            cache.remove(k);
            dropDeps(k);
            skyPairs.remove(k);
            ionoPairs.remove(k);
            int other = keyLo(k) == n.idx ? keyHi(k) : keyLo(k);
            Set<Long> os = pairsOf.get(other);
            if (os != null) os.remove(k);
        }
    }

    private void dropDeps(long key) {
        PairDeps pd = pairDeps.remove(key);
        if (pd == null) return;
        Map<Long, Set<Long>> m = depPairs.get(pd.dimension());
        if (m == null) return;
        for (long d : pd.deps()) {
            Set<Long> s = m.get(d);
            if (s != null && s.remove(key) && s.isEmpty()) m.remove(d);
        }
    }

    private void recompute(WorldAccess access, long gameTick) {
        while (!urgent.isEmpty() || !background.isEmpty()) {
            ArrayDeque<Long> q = urgent.isEmpty() ? background : urgent;
            long key = q.peekFirst();
            Node a = byIdx.get(keyLo(key)), b = byIdx.get(keyHi(key));
            if (a == null || b == null || a.removed || b.removed) {
                q.pollFirst();
                queued.remove(key);
                continue;
            }
            Pose pa = a.ep.pose(), pb = b.ep.pose();
            double d = pa == null || pb == null ? 0 : distance(pa, pb);
            int estimate = 2 + (int) (Math.min(d, 2 * PathTracer.VOXEL_END) * 2 / 32) + (d > 2 * PathTracer.VOXEL_END ? 8 : 0);
            if (budget.claim(gameTick, RadioConfig.raysPerTick(), estimate) < estimate && computedLastTick > 0) return;
            q.pollFirst();
            queued.remove(key);
            if (pa == null || pb == null || !pa.sameDimension(pb)) continue;
            RfWorld world = access.world(pa.dimension());
            if (world == null) continue;
            compute(key, a, pa, b, pb, world, access.conditions(pa.dimension()), gameTick);
            computedLastTick++;
            computedTotal++;
        }
    }

    private void compute(long key, Node a, Pose pa, Node b, Pose pb, RfWorld world, Conditions c, long gameTick) {
        int bin = keyBin(key);
        double f = frequencyFor(a, b, bin);
        double dx = pb.x() - pa.x(), dy = pb.y() - pa.y(), dz = pb.z() - pa.z();
        double d = Math.max(1e-6, Math.sqrt(dx * dx + dy * dy + dz * dz));
        double ux = dx / d, uy = dy / d, uz = dz / d;
        double[] la = pa.toLocal(ux, uy, uz), lb = pb.toLocal(-ux, -uy, -uz);
        double gA = a.ep.antenna().gainDbi(la[0], la[1], la[2]);
        double gB = b.ep.antenna().gainDbi(lb[0], lb[1], lb[2]);
        double[] ea = worldPol(pa, a.ep.antenna().polarization(la[0], la[1], la[2]));
        double[] eb = worldPol(pb, b.ep.antenna().polarization(lb[0], lb[1], lb[2]));
        double pol = polarizationLossDb(ea, eb, ux, uy, uz);
        Polarization groundPol = Math.abs(ea[1]) >= 0.7071 ? Polarization.VERTICAL : Polarization.HORIZONTAL;
        if (c == null) c = Conditions.DEFAULT;
        Ionosphere iono = f < PathTracer.UNDERGROUND_MAX_HZ ? ionosphere() : null;
        PathTracer.Sky sky = new PathTracer.Sky(iono, c.dayTime(), c.hasSky());
        if (iono != null && c.hasSky()) ionoPairs.put(key, pa.dimension());
        else ionoPairs.remove(key);
        List<Long> deps = new ArrayList<>();
        PathTracer.Result r = tracer.trace(world, pa.x(), pa.y(), pa.z(), pb.x(), pb.y(), pb.z(), f, groundPol, sky, deps::add);
        if (r.mode() == PathLossModel.Mode.SKYWAVE) {
            pol = PolarizationLoss.skywaveDb(groundPol, seed ^ key);
            skyPairs.add(key);
        } else {
            skyPairs.remove(key);
        }
        double k = r.lineOfSight() ? kLosLinear : 0;
        boolean aIsLo = a.idx < b.idx;
        LinkCache.Link link = new LinkCache.Link(r.excessDb(), f, aIsLo ? gA : gB, aIsLo ? gB : gA, pol, k,
                r.lineOfSight(), gameTick, r);
        cache.put(key, link);
        pairsOf.computeIfAbsent(a.idx, i -> new HashSet<>()).add(key);
        pairsOf.computeIfAbsent(b.idx, i -> new HashSet<>()).add(key);
        dropDeps(key);
        long[] arr = new long[deps.size()];
        Map<Long, Set<Long>> m = depPairs.computeIfAbsent(pa.dimension(), x -> new HashMap<>());
        for (int i = 0; i < arr.length; i++) {
            arr[i] = deps.get(i);
            m.computeIfAbsent(arr[i], x -> new HashSet<>()).add(key);
        }
        pairDeps.put(key, new PairDeps(pa.dimension(), arr));
    }

    private Ionosphere iono;
    private double ionoCompression = Double.NaN;

    private Ionosphere ionosphere() {
        double comp = RadioConfig.hopCompression();
        if (iono == null || comp != ionoCompression) {
            ionoCompression = comp;
            iono = Ionosphere.DEFAULT.withMaxHopBlocks(Math.max(1, Ionosphere.DEFAULT.maxHopRealM() / comp));
        }
        return iono;
    }

    /** The frequency a pair's trace in {@code bin} is computed at: a tuned or last transmit frequency in the bin, else its centre. */
    private static double frequencyFor(Node a, Node b, int bin) {
        for (Node n : new Node[] {a, b}) {
            Channel ch = n.ep.tunedChannel();
            if (ch != null && traceBin(ch.centerHz()) == bin) return ch.centerHz();
        }
        for (Node n : new Node[] {a, b})
            if (n.lastTxHz > 0 && traceBin(n.lastTxHz) == bin) return n.lastTxHz;
        return binCenterHz(bin);
    }

    private static double[] worldPol(Pose p, double[] local) {
        if (local == null || local.length < 3) return new double[] {0, 1, 0};
        return p.toWorld(local[0], local[1], local[2]);
    }

    /** Mismatch between two linear E-field directions as seen across the path (both projected off the path axis). */
    static double polarizationLossDb(double[] ea, double[] eb, double ux, double uy, double uz) {
        double[] a = perp(ea, ux, uy, uz), b = perp(eb, ux, uy, uz);
        double na = Math.sqrt(a[0] * a[0] + a[1] * a[1] + a[2] * a[2]), nb = Math.sqrt(b[0] * b[0] + b[1] * b[1] + b[2] * b[2]);
        if (na < 1e-6 || nb < 1e-6) return PolarizationLoss.CROSS_POL_CAP_DB;
        double cos = Math.abs(a[0] * b[0] + a[1] * b[1] + a[2] * b[2]) / (na * nb);
        return PolarizationLoss.db(Math.acos(Math.min(1, cos)));
    }

    private static double[] perp(double[] v, double ux, double uy, double uz) {
        double dot = v[0] * ux + v[1] * uy + v[2] * uz;
        return new double[] {v[0] - dot * ux, v[1] - dot * uy, v[2] - dot * uz};
    }

    // ------------------------------------------------------------------ world change hooks (server thread)

    /** A block changed: stale every pair whose trace crossed its section (and its column, if at the surface). */
    public void onBlockChanged(String dimension, int x, int y, int z, boolean atSurface) {
        staleDep(dimension, Sections.ofBlock(x, y, z));
        if (atSurface) staleDep(dimension, Sections.columnOfBlock(x, z));
    }

    /** A chunk loaded or unloaded (real blocks ↔ RF summary): stale everything traced through it. */
    public void onChunkChanged(String dimension, int cx, int cz, int minSectionY, int maxSectionY) {
        Map<Long, Set<Long>> m = depPairs.get(dimension);
        if (m == null) return;
        staleDep(dimension, Sections.column(cx, cz));
        for (int sy = minSectionY; sy <= maxSectionY; sy++) staleDep(dimension, Sections.key(cx, sy, cz));
    }

    /** A volume moved through a box (a Sable sub-level): stale every pair traced through it. */
    public void onRegionChanged(String dimension, double minX, double minY, double minZ, double maxX, double maxY, double maxZ) {
        Map<Long, Set<Long>> m = depPairs.get(dimension);
        if (m == null) return;
        int sx0 = (int) Math.floor(minX) >> 4, sx1 = (int) Math.floor(maxX) >> 4;
        int sy0 = (int) Math.floor(minY) >> 4, sy1 = (int) Math.floor(maxY) >> 4;
        int sz0 = (int) Math.floor(minZ) >> 4, sz1 = (int) Math.floor(maxZ) >> 4;
        if ((long) (sx1 - sx0 + 1) * (sy1 - sy0 + 1) * (sz1 - sz0 + 1) > 65536) {
            for (Set<Long> s : m.values()) for (Long k : s) enqueue(k, false);
            return;
        }
        for (int sx = sx0; sx <= sx1; sx++)
            for (int sz = sz0; sz <= sz1; sz++) {
                staleDep(dimension, Sections.column(sx, sz));
                for (int sy = sy0; sy <= sy1; sy++) staleDep(dimension, Sections.key(sx, sy, sz));
            }
    }

    private void staleDep(String dimension, long dep) {
        Map<Long, Set<Long>> m = depPairs.get(dimension);
        if (m == null) return;
        Set<Long> s = m.get(dep);
        if (s != null) for (Long k : s) enqueue(k, true);
    }

    // ------------------------------------------------------------------ introspection

    public int endpointCount() {
        return nodes.size();
    }

    public int cachedLinks() {
        return cache.size();
    }

    public int queuedLinks() {
        return queued.size();
    }

    public int computedLastTick() {
        return computedLastTick;
    }

    public long computedTotal() {
        return computedTotal;
    }

    /** Trace a path right now (debug command, server thread). */
    public PathTracer.Result traceNow(RfWorld world, Pose a, Pose b, double freqHz, Conditions c) {
        if (c == null) c = Conditions.DEFAULT;
        Ionosphere i = freqHz < PathTracer.UNDERGROUND_MAX_HZ ? ionosphere() : null;
        return tracer.trace(world, a.x(), a.y(), a.z(), b.x(), b.y(), b.z(), freqHz, Polarization.VERTICAL,
                new PathTracer.Sky(i, c.dayTime(), c.hasSky()), null);
    }

    static double distance(Pose a, Pose b) {
        double dx = a.x() - b.x(), dy = a.y() - b.y(), dz = a.z() - b.z();
        return Math.sqrt(dx * dx + dy * dy + dz * dz);
    }
}
