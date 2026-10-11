package com.example.evanscomputermod.radio.microwave;

import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;
import com.example.evanscomputermod.radio.medium.BasicRadioMedium;

import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.ThreadLocalRandom;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.LongSupplier;
import java.util.function.Supplier;

/**
 * The microwave radio's engine, free of Minecraft: a {@link RadioEndpoint} on
 * the shared medium that is also a layer-2 bridge between its cable segment
 * and whichever radio on the same band, width and channel it hears.
 *
 * <p><b>Bridging.</b> Every Ethernet frame seen on the cable (the radio is a
 * promiscuous bridge port) goes on the air as one {@link Emission#frame}
 * unless its destination was learned on the local side. Frames decoded from
 * the air go onto the local cable through {@link CablePort} (the hub's
 * {@code transmitFromPort}, which learns the far MACs behind this port). The
 * radio keeps its own local/remote MAC tables for that filtering.
 *
 * <p><b>Point to point.</b> A microwave link pairs exactly two radios. Each radio
 * picks one peer from the beacons it hears: the strongest radio that is free or
 * already pairs with it (beacons say whom their sender pairs with). Data frames
 * are only bridged between two radios that pair with each other, so a third radio
 * on the same channel never joins the bridge (no layer-2 loop); it shows as
 * unlinked until a free partner turns up.
 *
 * <p><b>Adaptive modulation.</b> Every 500 ms each radio beacons at MW-BPSK,
 * reporting the SINR it measured on the last frame from its peer. A sender
 * picks the fastest {@link MwModulation} that the peer's reported SINR carries
 * with 3 dB margin, falling back to BPSK when feedback is older than 2 s.
 *
 * <p><b>Atmosphere.</b> The medium doesn't know the weather, so the receiver
 * applies it: for each frame from another microwave radio it computes the
 * gaseous + rain loss over the actual path ({@link Atmosphere}, using the mean
 * of the two ends' rain rates), lowers RSSI and SINR by it, drops the frame
 * below sensitivity, and otherwise thins deliveries by the ratio of success
 * probabilities, which is exactly the frame-error rate at the reduced SINR.
 *
 * <p>Thread model: {@link #fromCable} runs on the sending computer's thread,
 * {@link #onReceive} on the transmitter's thread; both are lock-free apart from
 * the airtime scheduler. Frames from the air are delivered inline (low latency)
 * unless this thread is already delivering, in which case they wait for
 * {@link #tick} — so a bridging loop can't recurse.
 */
public final class MicrowaveLink implements RadioEndpoint {

    /** Where frames from the air go: the local cable segment. */
    public interface CablePort {
        void toCable(byte[] frame);
    }

    public static final double MIN_TX_DBM = -40, MAX_TX_DBM = 30, DEFAULT_TX_DBM = 20;
    public static final double NOISE_FIGURE_DB = 5;
    static final long BEACON_INTERVAL_US = 500_000;
    static final long FEEDBACK_STALE_US = 2_000_000;
    static final long FDB_AGE_MS = 300_000;
    static final double PREAMBLE_US = 2;
    static final int MAX_QUEUE = 1024, INLINE_BUDGET = 64;
    static final byte TYPE_DATA = 1, TYPE_BEACON = 2;

    /** Every live microwave radio, so a receiver can find the sender's pose and weather. */
    private static final Map<UUID, MicrowaveLink> ACTIVE = new ConcurrentHashMap<>();
    private static final ThreadLocal<Boolean> DELIVERING = ThreadLocal.withInitial(() -> false);

    private final UUID id;
    private final byte[] mac;
    private final Supplier<RadioMedium> medium;
    private final CablePort port;
    private final LongSupplier millis;

    private volatile MwBand band = MwBand.GHZ_24;
    private volatile int widthMhz = MwBand.GHZ_24.defaultWidthMhz;
    private volatile int channelNumber = 0;
    private volatile Channel channel = band.channel(0, widthMhz);
    private volatile double txPowerDbm = DEFAULT_TX_DBM;

    private volatile Pose pose;
    private volatile double dishDiameterM;
    private volatile DishPattern pattern;
    private volatile double rainRateMmPerH;

    private RadioMedium registeredWith;
    private long nextFreeMicros;
    private long lastBeaconMicros = Long.MIN_VALUE;
    private long ticks;

    private final ConcurrentLinkedQueue<byte[]> rxQueue = new ConcurrentLinkedQueue<>();
    private final AtomicInteger queued = new AtomicInteger();
    private final Map<Long, Long> localMacs = new ConcurrentHashMap<>();
    private final Map<Long, Long> remoteMacs = new ConcurrentHashMap<>();

    /** What one radio's beacons said, heard here: strength, when, and whom it pairs with (null = nobody). */
    private record Heard(double rssiDbm, long micros, UUID pairsWith) {}

    /** Beacons heard per sender, for picking the one peer (point-to-point). */
    private final Map<UUID, Heard> heardFrom = new ConcurrentHashMap<>();
    private volatile UUID peer;
    /** A new peer was chosen: beacon on the next tick so it learns at once that we pair with it. */
    private volatile boolean beaconSoon;
    private volatile double lastRssiDbm = Double.NaN, lastSinrDb = Double.NaN;
    private volatile long lastHeardMicros = Long.MIN_VALUE;
    private volatile double peerReportedSinrDb = Double.NaN;
    private volatile long feedbackMicros = Long.MIN_VALUE;
    private volatile MwModulation lastTxModulation = MwModulation.BPSK;
    private volatile double lastAtmosphereDb;

    private final AtomicLong txFrames = new AtomicLong(), rxFrames = new AtomicLong(), txBytes = new AtomicLong(),
            rxBytes = new AtomicLong(), filtered = new AtomicLong(), faded = new AtomicLong(), beaconsHeard = new AtomicLong(),
            strayFrames = new AtomicLong();

    public MicrowaveLink(UUID id, byte[] mac, Supplier<RadioMedium> medium, CablePort port, LongSupplier millis) {
        this.id = id;
        this.mac = mac.clone();
        this.medium = medium;
        this.port = port;
        this.millis = millis;
    }

    /** The live radio with this id (it keeps its id when a Sable ship assembly moves its block), or null. */
    public static MicrowaveLink find(UUID id) {
        return ACTIVE.get(id);
    }

    // ------------------------------------------------------------ configuration

    /** Set band, channel width and channel together (validated; nothing changes on error). */
    public synchronized void configure(MwBand band, int widthMhz, int channelNumber) {
        Channel ch = band.channel(channelNumber, widthMhz);
        this.band = band;
        this.widthMhz = widthMhz;
        this.channelNumber = channelNumber;
        this.channel = ch;
        rebuildPattern();
        peerReportedSinrDb = Double.NaN;
        RadioMedium m = registeredWith;
        if (m != null) m.invalidate(this);
    }

    /** Switch band, keeping the width if the new band has it (else its default) and the channel if it fits (else 0). */
    public void setBand(MwBand b) {
        int w = b.allowsWidth(widthMhz) ? widthMhz : b.defaultWidthMhz;
        int c = channelNumber < b.channelCount(w) ? channelNumber : 0;
        configure(b, w, c);
    }

    public void setWidthMhz(int w) {
        configure(band, w, channelNumber < band.channelCount(w) ? channelNumber : 0);
    }

    public void setChannelNumber(int n) {
        configure(band, widthMhz, n);
    }

    public void setTxPowerDbm(double dbm) {
        if (!(dbm >= MIN_TX_DBM && dbm <= MAX_TX_DBM))
            throw new IllegalArgumentException("tx power must be " + MIN_TX_DBM + " to " + MAX_TX_DBM + " dBm");
        txPowerDbm = dbm;
    }

    public MwBand band() { return band; }
    public int widthMhz() { return widthMhz; }
    public int channelNumber() { return channelNumber; }
    public Channel channel() { return channel; }
    public double txPowerDbm() { return txPowerDbm; }
    public byte[] mac() { return mac.clone(); }
    public UUID peer() { return peer; }
    public double lastRssiDbm() { return lastRssiDbm; }
    public double lastSinrDb() { return lastSinrDb; }
    public long txFrames() { return txFrames.get(); }
    public long rxFrames() { return rxFrames.get(); }
    public double rainRateMmPerH() { return rainRateMmPerH; }

    // ------------------------------------------------------------ antenna and pose (owner, server thread)

    /**
     * The dish this radio feeds: its world pose (boresight = local +Z) and
     * diameter, or {@code null}/0 when no dish is connected. A new or removed
     * dish invalidates the medium's links; turning or moving it doesn't need to
     * (the medium watches poses, refreshing the narrow beam's gains after a
     * quarter of the usual turn threshold, see {@link #turnThresholdScale}).
     */
    public void setDish(Pose dishPose, double diameterM) {
        Pose before = pose;
        boolean antennaChanged = diameterM != dishDiameterM;
        pose = dishPose;
        if (antennaChanged) {
            dishDiameterM = diameterM;
            rebuildPattern();
        }
        RadioMedium m = registeredWith;
        if (m != null && (antennaChanged || (before == null) != (dishPose == null))) m.invalidate(this);
    }

    @Override
    public double turnThresholdScale() {
        return 0.25;
    }

    private void rebuildPattern() {
        double d = dishDiameterM;
        pattern = d > 0 ? new DishPattern(d, channel.centerHz()) : null;
    }

    /** Rain rate at this end, mm/h (from Minecraft weather and biome; see {@link Atmosphere#rainRateMmPerH}). */
    public void setRainRate(double mmPerH) {
        rainRateMmPerH = Math.max(0, mmPerH);
    }

    public DishPattern dish() {
        return pattern;
    }

    /** Has a dish and a pose, so it can transmit and listen. */
    public boolean ready() {
        return pattern != null && pose != null;
    }

    // ------------------------------------------------------------ lifecycle

    /** Register with the running medium (re-registering if it was replaced) and join the live-radio table. */
    public void tick() {
        RadioMedium m = medium.get();
        if (m != registeredWith) {
            if (registeredWith != null) registeredWith.unregister(this);
            if (m != null) m.register(this);
            registeredWith = m;
        }
        ACTIVE.put(id, this);
        if (m == null) return;
        long now = m.nowMicros();
        if (ready() && (beaconSoon || lastBeaconMicros == Long.MIN_VALUE || now - lastBeaconMicros >= BEACON_INTERVAL_US)) {
            beaconSoon = false;
            lastBeaconMicros = now;
            sendBeacon(m);
        }
        deliver(MAX_QUEUE);
        if (++ticks % 200 == 0) pruneTables();
    }

    /** Leave the medium and the live table (block removed or chunk unloaded). */
    public void stop() {
        if (registeredWith != null) registeredWith.unregister(this);
        registeredWith = null;
        ACTIVE.remove(id, this);
        rxQueue.clear();
        queued.set(0);
    }

    // ------------------------------------------------------------ cable -> air

    /** A frame seen on the local cable segment (bridge-port sink). */
    public void fromCable(byte[] frame) {
        if (frame.length < 14) return;
        long now = millis.getAsLong();
        long src = macKey(frame, 6);
        if ((frame[6] & 1) == 0) {
            localMacs.put(src, now);
            remoteMacs.remove(src);
        }
        if ((frame[0] & 1) == 0) {
            long dst = macKey(frame, 0);
            Long local = localMacs.get(dst);
            if (local != null && now - local < FDB_AGE_MS && !remoteMacs.containsKey(dst)) {
                filtered.incrementAndGet();
                return;
            }
        }
        RadioMedium m = registeredWith;
        if (m == null || !ready()) return;
        byte[] payload = new byte[4 + frame.length];
        payload[0] = 'M';
        payload[1] = 'W';
        payload[2] = TYPE_DATA;
        payload[3] = 1;
        System.arraycopy(frame, 0, payload, 4, frame.length);
        MwModulation mod = modulationFor(payload.length * 8, m.nowMicros());
        lastTxModulation = mod;
        emit(m, payload, mod);
        txFrames.incrementAndGet();
        txBytes.addAndGet(frame.length);
    }

    /** The modulation a frame of {@code bits} goes out at, from the peer's SINR feedback. */
    MwModulation modulationFor(int bits, long nowMicros) {
        if (feedbackMicros == Long.MIN_VALUE || nowMicros - feedbackMicros > FEEDBACK_STALE_US) return MwModulation.BPSK;
        return MwModulation.choose(peerReportedSinrDb, bits);
    }

    private void sendBeacon(RadioMedium m) {
        ByteBuffer b = ByteBuffer.allocate(30);
        b.put((byte) 'M').put((byte) 'W').put(TYPE_BEACON).put((byte) 1);
        UUID p = peer;
        boolean fresh = p != null && lastHeardMicros != Long.MIN_VALUE && m.nowMicros() - lastHeardMicros <= FEEDBACK_STALE_US;
        b.putFloat(fresh ? (float) lastSinrDb : Float.NaN);
        b.putLong(p == null ? 0 : p.getMostSignificantBits()).putLong(p == null ? 0 : p.getLeastSignificantBits());
        b.put(mac);
        emit(m, b.array(), MwModulation.BPSK);
    }

    private void emit(RadioMedium m, byte[] payload, MwModulation mod) {
        Channel ch = channel;
        double rate = mod.bitRate(ch.bandwidthHz());
        long dur = Math.max(1, (long) Math.ceil(PREAMBLE_US + payload.length * 8 / rate * 1e6));
        long start;
        synchronized (this) {
            start = Math.max(m.nowMicros(), nextFreeMicros);
            nextFreeMicros = start + dur;
        }
        m.transmit(this, Emission.frame(ch, txPowerDbm, start, dur, mod.id, rate, payload));
    }

    // ------------------------------------------------------------ air -> cable

    @Override
    public void onReceive(Reception r) {
        byte[] p = r.payload();
        if (p == null || p.length < 4 || p[0] != 'M' || p[1] != 'W') return;
        Channel ch = channel;
        Channel got = r.emission().channel();
        if (Math.abs(got.centerHz() - ch.centerHz()) > 1 || Math.abs(got.bandwidthHz() - ch.bandwidthHz()) > 1) return;
        MicrowaveLink from = ACTIVE.get(r.from());
        Pose here = pose;
        double loss = 0;
        if (from != null && from.pose != null && here != null)
            loss = Atmosphere.lossDb(ch.centerHz(), here.distanceTo(from.pose), (rainRateMmPerH + from.rainRateMmPerH) / 2);
        double rssi = r.rssiDbm() - loss, sinr = r.sinrDb() - loss;
        if (rssi < sensitivityDbm()) {
            faded.incrementAndGet();
            return;
        }
        if (loss > 0) {
            int bits = p.length * 8;
            String mod = r.emission().modulation();
            double okBefore = 1 - BasicRadioMedium.packetErrorRate(mod, r.sinrDb(), bits);
            double okAfter = 1 - BasicRadioMedium.packetErrorRate(mod, sinr, bits);
            if (okBefore > 0 && ThreadLocalRandom.current().nextDouble() * okBefore > okAfter) {
                faded.incrementAndGet();
                return;
            }
        }
        UUID sender = r.from();
        if (p[2] == TYPE_BEACON && p.length >= 30) {
            beaconsHeard.incrementAndGet();
            ByteBuffer b = ByteBuffer.wrap(p);
            float reported = b.getFloat(4);
            long hi = b.getLong(8), lo = b.getLong(16);
            UUID target = hi == 0 && lo == 0 ? null : new UUID(hi, lo);
            heardFrom.put(sender, new Heard(rssi, r.timestampMicros(), target));
            choosePeer(r.timestampMicros());
            if (!sender.equals(peer)) return;
            noteFromPeer(loss, rssi, sinr, r.timestampMicros());
            if (id.equals(target) && !Float.isNaN(reported)) {
                peerReportedSinrDb = reported;
                feedbackMicros = r.timestampMicros();
            }
        } else if (p[2] == TYPE_DATA && p.length >= 4 + 14) {
            if (!pairedWith(sender, r.timestampMicros())) {
                strayFrames.incrementAndGet();   // a radio we don't pair with (or that pairs with someone else)
                return;
            }
            noteFromPeer(loss, rssi, sinr, r.timestampMicros());
            rxFrames.incrementAndGet();
            rxBytes.addAndGet(p.length - 4);
            if (queued.incrementAndGet() > MAX_QUEUE) {
                queued.decrementAndGet();
                return;   // overrun: drop, as a real bridge would
            }
            rxQueue.add(Arrays.copyOfRange(p, 4, p.length));
            deliver(INLINE_BUDGET);
        }
    }

    private void noteFromPeer(double atmosphereDb, double rssi, double sinr, long micros) {
        lastAtmosphereDb = atmosphereDb;
        lastRssiDbm = rssi;
        lastSinrDb = sinr;
        lastHeardMicros = micros;
    }

    /**
     * Pick the peer: the strongest radio heard recently that is free or already pairs with
     * this one; if every radio heard pairs with someone else, none.
     */
    private synchronized void choosePeer(long nowMicros) {
        UUID best = null;
        double bestRssi = Double.NEGATIVE_INFINITY;
        for (Map.Entry<UUID, Heard> e : heardFrom.entrySet()) {
            Heard h = e.getValue();
            if (nowMicros - h.micros() > FEEDBACK_STALE_US) continue;
            if (h.pairsWith() != null && !h.pairsWith().equals(id)) continue;   // taken by another radio
            if (h.rssiDbm() > bestRssi) {
                bestRssi = h.rssiDbm();
                best = e.getKey();
            }
        }
        if (best != null && !best.equals(peer)) {
            peer = best;
            beaconSoon = true;
            peerReportedSinrDb = Double.NaN;
            feedbackMicros = Long.MIN_VALUE;
        } else if (best == null && peer != null) {
            Heard h = heardFrom.get(peer);
            if (h == null || nowMicros - h.micros() > FEEDBACK_STALE_US || (h.pairsWith() != null && !h.pairsWith().equals(id)))
                peer = null;
        }
    }

    /** True if {@code from} is this radio's peer and its latest beacon pairs it with this radio (or nobody yet). */
    boolean pairedWith(UUID from, long nowMicros) {
        if (!from.equals(peer)) return false;
        Heard h = heardFrom.get(from);
        return h != null && nowMicros - h.micros() <= 2 * FEEDBACK_STALE_US && (h.pairsWith() == null || h.pairsWith().equals(id));
    }

    private void deliver(int budget) {
        if (DELIVERING.get()) return;
        DELIVERING.set(true);
        try {
            for (int i = 0; i < budget; i++) {
                byte[] f = rxQueue.poll();
                if (f == null) break;
                queued.decrementAndGet();
                if ((f[6] & 1) == 0) {
                    long src = macKey(f, 6);
                    remoteMacs.put(src, millis.getAsLong());
                    localMacs.remove(src);
                }
                port.toCable(f);
            }
        } finally {
            DELIVERING.set(false);
        }
    }

    private void pruneTables() {
        long now = millis.getAsLong();
        localMacs.values().removeIf(t -> now - t > FDB_AGE_MS);
        remoteMacs.values().removeIf(t -> now - t > FDB_AGE_MS);
        RadioMedium m = registeredWith;
        if (m != null) {
            long us = m.nowMicros();
            heardFrom.values().removeIf(h -> us - h.micros() > 10 * FEEDBACK_STALE_US);
        }
    }

    static long macKey(byte[] f, int off) {
        long k = 0;
        for (int i = 0; i < 6; i++) k = (k << 8) | (f[off + i] & 0xff);
        return k;
    }

    // ------------------------------------------------------------ alignment

    /**
     * Power this radio would receive from {@code from} if its dish had pose
     * {@code candidate} (dBm): far tx power, both dish gains, the medium's
     * cached path gain (free space if it has none yet) and the atmosphere.
     */
    public double predictedRxDbm(MicrowaveLink from, Pose candidate, RadioMedium m) {
        Pose there = from.pose;
        DishPattern mine = pattern, theirs = from.pattern;
        if (there == null || candidate == null || mine == null || theirs == null || !there.sameDimension(candidate))
            return Double.NEGATIVE_INFINITY;
        double dx = candidate.x() - there.x(), dy = candidate.y() - there.y(), dz = candidate.z() - there.z();
        double d = Math.max(1, Math.sqrt(dx * dx + dy * dy + dz * dz));
        double f = channel.centerHz();
        double[] outTx = there.toLocal(dx / d, dy / d, dz / d);
        double[] inRx = candidate.toLocal(-dx / d, -dy / d, -dz / d);
        double path = m == null ? Double.NaN : m.pathGainDb(from, this, f);
        if (Double.isNaN(path)) path = -BasicRadioMedium.freeSpaceLossDb(d, f);
        return from.txPowerDbm + theirs.gainDbi(outTx[0], outTx[1], outTx[2]) + mine.gainDbi(inRx[0], inRx[1], inRx[2])
                + path - Atmosphere.lossDb(f, d, (rainRateMmPerH + from.rainRateMmPerH) / 2);
    }

    /** Other live radios on exactly this band, width and channel (what an alignment scan can hear). */
    public List<MicrowaveLink> sameChannelRadios() {
        List<MicrowaveLink> out = new ArrayList<>();
        Channel ch = channel;
        for (MicrowaveLink l : ACTIVE.values()) {
            if (l == this || !l.ready()) continue;
            Channel c = l.channel;
            if (Math.abs(c.centerHz() - ch.centerHz()) < 1 && Math.abs(c.bandwidthHz() - ch.bandwidthHz()) < 1) out.add(l);
        }
        return out;
    }

    // ------------------------------------------------------------ status

    /** Current state for the peripheral and the status message. */
    public Map<String, Object> status() {
        Map<String, Object> s = new LinkedHashMap<>();
        Channel ch = channel;
        RadioMedium m = registeredWith;
        long now = m == null ? 0 : m.nowMicros();
        boolean linked = lastHeardMicros != Long.MIN_VALUE && now - lastHeardMicros <= FEEDBACK_STALE_US;
        MwModulation mod = modulationFor(1500 * 8, now);
        s.put("mac", formatMac(mac));
        s.put("band_ghz", band.ghz);
        s.put("channel", channelNumber);
        s.put("bandwidth_mhz", widthMhz);
        s.put("frequency_mhz", ch.centerHz() / 1e6);
        s.put("tx_power_dbm", txPowerDbm);
        DishPattern d = pattern;
        s.put("dish", d != null);
        s.put("dish_gain_dbi", d == null ? 0.0 : d.peakGainDbi());
        s.put("beamwidth_deg", d == null ? 0.0 : d.beamwidthDeg());
        s.put("linked", linked);
        s.put("peer", peer == null ? "" : peer.toString());
        s.put("rssi_dbm", linked ? lastRssiDbm : Double.NaN);
        s.put("sinr_db", linked ? lastSinrDb : Double.NaN);
        s.put("modulation", mod.id);
        s.put("rate_mbps", mod.bitRate(ch.bandwidthHz()) / 1e6);
        s.put("atmosphere_db", lastAtmosphereDb);
        s.put("rain_mm_h", rainRateMmPerH);
        s.put("tx_frames", txFrames.get());
        s.put("rx_frames", rxFrames.get());
        s.put("tx_bytes", txBytes.get());
        s.put("rx_bytes", rxBytes.get());
        s.put("filtered_frames", filtered.get());
        s.put("faded_frames", faded.get());
        s.put("stray_frames", strayFrames.get());
        return s;
    }

    static String formatMac(byte[] m) {
        return String.format("%02x:%02x:%02x:%02x:%02x:%02x", m[0] & 0xff, m[1] & 0xff, m[2] & 0xff, m[3] & 0xff, m[4] & 0xff, m[5] & 0xff);
    }

    // ------------------------------------------------------------ RadioEndpoint

    @Override public UUID id() { return id; }
    @Override public Pose pose() { return pose; }

    @Override
    public AntennaPattern antenna() {
        DishPattern d = pattern;
        return d == null ? AntennaPattern.ISOTROPIC : d;
    }

    @Override
    public Channel tunedChannel() {
        return ready() ? channel : null;
    }

    @Override public double noiseFigureDb() { return NOISE_FIGURE_DB; }

    @Override
    public double sensitivityDbm() {
        return BasicRadioMedium.noiseDbm(channel.bandwidthHz(), NOISE_FIGURE_DB) - 3;
    }

    @Override public double maxTxPowerDbm() { return txPowerDbm; }
}
