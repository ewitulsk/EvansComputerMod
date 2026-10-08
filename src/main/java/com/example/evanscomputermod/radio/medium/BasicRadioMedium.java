package com.example.evanscomputermod.radio.medium;

import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ConcurrentLinkedDeque;
import java.util.function.LongSupplier;

/**
 * The reference {@link RadioMedium}: free-space link budgets with antenna gains,
 * an airtime clock, overlap-weighted interference and one deterministic
 * packet-error roll per frame and receiver. Pure Java (no world access), so it
 * is unit-testable; the in-world medium extends it with the cached voxel /
 * terrain path loss through {@link #extraPathLossDb}.
 */
public class BasicRadioMedium implements RadioMedium {

    /** Boltzmann noise density at 290 K, dBm/Hz. */
    public static final double THERMAL_DBM_PER_HZ = -174;

    private final LongSupplier clockMicros;
    private final Map<UUID, RadioEndpoint> endpoints = new ConcurrentHashMap<>();
    private final ConcurrentLinkedDeque<Active> active = new ConcurrentLinkedDeque<>();
    private final long seed;

    private record Active(UUID from, Emission emission, Pose pose, RadioEndpoint sender) {}

    public BasicRadioMedium(LongSupplier clockMicros, long seed) {
        this.clockMicros = clockMicros;
        this.seed = seed;
    }

    @Override
    public long nowMicros() {
        return clockMicros.getAsLong();
    }

    @Override
    public void register(RadioEndpoint endpoint) {
        endpoints.put(endpoint.id(), endpoint);
    }

    @Override
    public void unregister(RadioEndpoint endpoint) {
        endpoints.remove(endpoint.id());
    }

    @Override
    public void invalidate(RadioEndpoint endpoint) {
        // Nothing cached in the reference medium.
    }

    public int endpointCount() {
        return endpoints.size();
    }

    @Override
    public Emission transmit(RadioEndpoint from, Emission e) {
        long now = nowMicros();
        if (e.startMicros() < now) {
            e = new Emission(e.kind(), e.channel(), e.powerDbm(), now, e.durationMicros(), e.modulation(),
                    e.bitRate(), e.payload(), e.iq(), e.sampleRateHz());
        }
        prune(now);
        Pose txPose = from.pose();
        Active self = new Active(from.id(), e, txPose, from);
        active.add(self);
        if (e.kind() != Emission.Kind.FRAME) return e;

        for (RadioEndpoint rx : endpoints.values()) {
            if (rx.id().equals(from.id()) || !rx.listening()) continue;
            Channel tuned = rx.tunedChannel();
            if (tuned == null || !tuned.overlaps(e.channel())) continue;
            Pose rxPose = rx.pose();
            if (!rxPose.sameDimension(txPose)) continue;
            double rssi = receivedDbm(from, txPose, e, rx, rxPose);
            if (rssi < rx.sensitivityDbm()) continue;
            double noiseMw = dbmToMw(noiseDbm(tuned.bandwidthHz(), rx.noiseFigureDb()));
            double interfMw = 0;
            for (Active a : active) {
                if (a == self || a.from.equals(rx.id()) || !a.emission.overlapsInTime(e)) continue;
                double w = overlapFraction(a.emission.channel(), tuned);
                if (w <= 0 || !a.pose.sameDimension(rxPose)) continue;
                interfMw += w * dbmToMw(receivedDbm(a.sender, a.pose, a.emission, rx, rxPose));
            }
            double sinr = rssi - mwToDbm(noiseMw + interfMw);
            double per = packetErrorRate(e.modulation(), sinr, e.payload() == null ? 0 : e.payload().length * 8);
            if (roll(e, rx.id()) < per) continue;
            rx.onReceive(new Reception(from.id(), e, rssi, sinr, e.endMicros()));
        }
        return e;
    }

    @Override
    public double channelPowerDbm(RadioEndpoint at, Channel channel) {
        long now = nowMicros();
        prune(now);
        Pose rxPose = at.pose();
        double mw = 0;
        for (Active a : active) {
            if (a.from.equals(at.id()) || a.emission.startMicros() > now || a.emission.endMicros() < now) continue;
            double w = overlapFraction(a.emission.channel(), channel);
            if (w <= 0 || !a.pose.sameDimension(rxPose)) continue;
            mw += w * dbmToMw(receivedDbm(a.sender, a.pose, a.emission, at, rxPose));
        }
        return mw <= 0 ? Double.NEGATIVE_INFINITY : mwToDbm(mw);
    }

    @Override
    public double pathGainDb(RadioEndpoint a, RadioEndpoint b, double freqHz) {
        Pose pa = a.pose(), pb = b.pose();
        if (!pa.sameDimension(pb)) return Double.NEGATIVE_INFINITY;
        return -freeSpaceLossDb(Math.max(1, pa.distanceTo(pb)), freqHz) - extraPathLossDb(a, pa, b, pb, freqHz);
    }

    /** Power at {@code rx} from an emission sent by {@code tx}, dBm. */
    protected double receivedDbm(RadioEndpoint tx, Pose txPose, Emission e, RadioEndpoint rx, Pose rxPose) {
        double dx = rxPose.x() - txPose.x(), dy = rxPose.y() - txPose.y(), dz = rxPose.z() - txPose.z();
        double d = Math.max(1, Math.sqrt(dx * dx + dy * dy + dz * dz));
        double f = e.channel().centerHz();
        double[] outTx = txPose.toLocal(dx / d, dy / d, dz / d);
        double[] outRx = rxPose.toLocal(-dx / d, -dy / d, -dz / d);
        double gtx = tx.antenna().gainDbi(outTx[0], outTx[1], outTx[2]) - tx.antenna().feedLossDb();
        double grx = rx.antenna().gainDbi(outRx[0], outRx[1], outRx[2]) - rx.antenna().feedLossDb();
        return e.powerDbm() + gtx + grx - freeSpaceLossDb(d, f) - extraPathLossDb(tx, txPose, rx, rxPose, f);
    }

    /** Losses beyond free space (walls, terrain, polarization). The reference medium has none. */
    protected double extraPathLossDb(RadioEndpoint tx, Pose txPose, RadioEndpoint rx, Pose rxPose, double freqHz) {
        return 0;
    }

    private void prune(long now) {
        // Keep emissions for 1 s after they end so late-starting overlaps still see them.
        active.removeIf(a -> a.emission.endMicros() < now - 1_000_000);
    }

    private double roll(Emission e, UUID rx) {
        long h = seed ^ e.startMicros() * 0x9E3779B97F4A7C15L ^ rx.getMostSignificantBits() ^ rx.getLeastSignificantBits() * 31;
        if (e.payload() != null) h ^= java.util.Arrays.hashCode(e.payload());
        h ^= h >>> 33; h *= 0xff51afd7ed558ccdL; h ^= h >>> 33; h *= 0xc4ceb9fe1a85ec53L; h ^= h >>> 33;
        return (h >>> 11) * 0x1.0p-53;
    }

    // ---- shared helpers (also used by tests and the in-world medium)

    public static double freeSpaceLossDb(double distanceM, double freqHz) {
        return 20 * Math.log10(distanceM) + 20 * Math.log10(freqHz) - 147.55;
    }

    public static double noiseDbm(double bandwidthHz, double noiseFigureDb) {
        return THERMAL_DBM_PER_HZ + 10 * Math.log10(bandwidthHz) + noiseFigureDb;
    }

    public static double dbmToMw(double dbm) {
        return Math.pow(10, dbm / 10);
    }

    public static double mwToDbm(double mw) {
        return 10 * Math.log10(mw);
    }

    /** Fraction of {@code a}'s power that falls inside {@code rx}'s passband (flat-spectrum approximation). */
    public static double overlapFraction(Channel a, Channel rx) {
        double lo = Math.max(a.lowHz(), rx.lowHz()), hi = Math.min(a.highHz(), rx.highHz());
        return hi <= lo ? 0 : (hi - lo) / a.bandwidthHz();
    }

    /**
     * Packet error rate from SINR for a named modulation: a steep logistic around
     * the modulation's required SINR (10% PER point), widened by packet length.
     */
    public static double packetErrorRate(String modulation, double sinrDb, int bits) {
        double required = requiredSinrDb(modulation);
        double lengthPenalty = bits > 0 ? Math.log10(Math.max(1, bits / 1000.0)) : 0;
        double x = sinrDb - required - lengthPenalty;
        return 1 / (1 + Math.exp(2.2 * x + Math.log(9)));   // PER = 0.1 at x = 0
    }

    /** SINR needed for ~10% PER on a 1000-bit frame. */
    public static double requiredSinrDb(String modulation) {
        if (modulation == null) return 10;
        String m = modulation.toUpperCase(java.util.Locale.ROOT);
        if (m.startsWith("DSSS-1")) return 0;
        if (m.startsWith("DSSS-2")) return 3;
        if (m.startsWith("CCK-5.5")) return 6;
        if (m.startsWith("CCK-11")) return 9;
        if (m.startsWith("OFDM-6")) return 5;
        if (m.startsWith("OFDM-9")) return 6;
        if (m.startsWith("OFDM-12")) return 8;
        if (m.startsWith("OFDM-18")) return 10;
        if (m.startsWith("OFDM-24")) return 13;
        if (m.startsWith("OFDM-36")) return 17;
        if (m.startsWith("OFDM-48")) return 21;
        if (m.startsWith("OFDM-54")) return 23;
        if (m.startsWith("HT-MCS")) {
            int mcs = Integer.parseInt(m.substring(6).replaceAll("[^0-9].*", ""));
            double[] t = {5, 8, 11, 14, 18, 22, 24, 26};
            return t[Math.min(7, mcs % 8)];
        }
        if (m.startsWith("CHIRP")) return -20;
        if (m.startsWith("AFSK") || m.startsWith("FSK")) return 10;
        if (m.startsWith("BPSK")) return 5;
        if (m.startsWith("QPSK")) return 8;
        if (m.startsWith("CTRL")) return 2;   // controller reports: robust low-rate GFSK-like
        return 10;
    }

    /** Snapshot of endpoints (tests, debug commands). */
    public List<RadioEndpoint> endpoints() {
        return new ArrayList<>(endpoints.values());
    }
}
