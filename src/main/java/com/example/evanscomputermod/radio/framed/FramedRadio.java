package com.example.evanscomputermod.radio.framed;

import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;
import com.example.evanscomputermod.radio.medium.BasicRadioMedium;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.IdentityHashMap;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.CopyOnWriteArraySet;

/**
 * The framed PHY radio: the "silicon" of a cellular radio, reusable by any host
 * (the Advanced SDR's framed mode, a modem module). The computer picks a
 * {@link PhyProfile}, a role, channel, bandwidth and power, and moves whole frames
 * with measurements through the device file ({@link FramedRecords}); the radio
 * does the per-frame work real basebands do in hardware:
 *
 * <ul>
 *   <li><b>Slots.</b> Transmissions start on slot boundaries of the medium's airtime
 *       clock ({@link PhyProfile#slotUs}): the next one, or a slot the program names
 *       (up to 1 s ahead).</li>
 *   <li><b>Link adaptation.</b> A unicast frame goes at the CQI the destination last
 *       reported (every frame header and ACK carries the sender's measured CQI of the
 *       peer it is addressed to); broadcasts, beacons and unknown peers use CQI 1. Each
 *       CQI is a {@code CELL-CQI<n>} emission whose error rate the medium takes from
 *       {@link CqiTable}.</li>
 *   <li><b>HARQ.</b> A frame sent with the HARQ flag is acknowledged by the receiving
 *       radio {@link PhyProfile#HARQ_ACK_SLOTS} slots after it started (an ACK frame on the
 *       air, which can be lost too); without an ACK {@link PhyProfile#HARQ_RETX_SLOTS}
 *       slots after a try it is sent again, two CQI steps lower, up to
 *       {@link PhyProfile#HARQ_MAX_RETX} times, then reported failed. Duplicates of a
 *       frame already delivered are acknowledged but not delivered again.</li>
 *   <li><b>Timing advance.</b> A UE measures the propagation delay of frames from base
 *       stations and starts its own transmissions that much early, so they reach the base
 *       station on the slot boundary.</li>
 *   <li><b>Beacon offload.</b> Up to {@link #BEACONS} frames repeated every N ms on the
 *       airtime clock without the program sending each one.</li>
 *   <li><b>Collisions.</b> A received frame is held until it has ended on the airtime
 *       clock; then every emission that overlapped it but was not on the air when the
 *       medium delivered it (a transmitter that keyed later in the same slot) is added to
 *       its interference and the packet-error roll is taken again.</li>
 * </ul>
 *
 * <p>Pure Java (no Minecraft): the host supplies the endpoint, medium, power cap and
 * transmit chain. Timers run in {@link #pump()}, which the host calls every server tick
 * and the device file calls on every access, so ACKs and retransmissions go out at the
 * first pump after their slot (beacons are scheduled up to {@link #LEAD_US} ahead, so
 * they keep exact timing).
 */
public final class FramedRadio {

    public enum Role { BS, UE }

    public static final int BROADCAST = 0xFFFFFFFF;
    public static final int MAX_PAYLOAD = 1500;
    public static final int BEACONS = 4;
    public static final int MAX_QUEUE = 256;
    /** Scheduled transmissions may name a slot at most this far ahead, µs. */
    public static final long MAX_AHEAD_US = 1_000_000;
    /** Beacons are put on the air up to this far ahead of their start, µs. */
    public static final long LEAD_US = 150_000;
    public static final long MIN_BEACON_MS = 5, MAX_BEACON_MS = 10_000;
    /** Air header: magic, version, flags, cqi_report, src, dst, seq, reserved. */
    static final int AIR_HEADER = 16;
    static final int MAGIC = 0xF7, VERSION = 1;

    /** What a host provides. Called with the radio's lock released except {@link #maxTxDbm} / {@link #noiseFigureDb}. */
    public interface Host {
        /** The endpoint on the medium whose tuning, antenna and listening follow this radio while it is active. */
        RadioEndpoint endpoint();

        /** The medium (null while there is no server). */
        RadioMedium medium();

        /** Transmit power the hardware can give, dBm (the exciter or module, an amplifier chain may raise it). */
        double maxTxDbm();

        double noiseFigureDb();

        /** The transmit gate and chain (server rules, amplifier): the emission as it goes out, or null if blocked. */
        default Emission beforeTransmit(Emission e) {
            return e;
        }

        /** A peripheral event for attached computers ({@code framed_rx}). */
        default void event(String name) {}

        /**
         * A page for the identity the radio watches ({@code wake tmsi N}) arrived:
         * wake the device's computer if it sleeps (any thread).
         */
        default void page() {}
    }

    private record Peer(double rssiDbm, double sinrDb, int cqi, int reported, long seenUs, long delayUs, long frames) {}

    private record Held(Reception r, byte[] air, Set<Emission> known) {}

    private static final class Harq {
        final int seq, dst, tag, sub;
        final byte[] payload;
        int attempts, cqi;
        long slot, deadlineUs, airUs;
        boolean acked;

        Harq(int seq, int dst, int tag, int sub, byte[] payload) {
            this.seq = seq;
            this.dst = dst;
            this.tag = tag;
            this.sub = sub;
            this.payload = payload;
        }
    }

    private static final class Beacon {
        byte[] payload;
        long periodUs, nextUs;
        int cqi, sub, seq;
    }

    private final Host host;
    private volatile boolean active;
    private volatile PhyProfile profile = PhyProfile.LTE;
    private volatile Role role = Role.UE;
    private volatile int channel = 1;
    private volatile double bandwidthHz = PhyProfile.LTE.bandwidthHz;
    private volatile double powerDbm = Double.NaN;   // NaN = the profile's default for the role
    private volatile int address;
    private volatile boolean promiscuous;

    private final ConcurrentLinkedQueue<Held> inbox = new ConcurrentLinkedQueue<>();
    private final List<Held> held = new ArrayList<>();
    private final ArrayDeque<byte[]> records = new ArrayDeque<>();
    private final Map<Integer, Peer> peers = new HashMap<>();
    private final Map<Integer, Integer> lastSeq = new HashMap<>();
    private final List<Harq> harq = new ArrayList<>();
    private final Beacon[] beacons = new Beacon[BEACONS];
    private final Set<Object> waiters = new CopyOnWriteArraySet<>();
    private int seq;
    private volatile long taUs;
    private long txFrames, txBytes, rxFrames, rxBytes, rxCollisions, rxDropped, harqRetx, harqFailed, harqAcked, beaconsSent, blocked, filtered;

    public FramedRadio(Host host, UUID id) {
        this.host = host;
        int a = (int) (id.getMostSignificantBits() ^ id.getLeastSignificantBits() ^ (id.getLeastSignificantBits() >>> 32)) & 0x7fffffff;
        this.address = a == 0 ? 1 : a;
    }

    // ------------------------------------------------------------ configuration

    public boolean active() { return active; }
    public PhyProfile profile() { return profile; }
    public Role role() { return role; }
    public int channel() { return channel; }
    public double bandwidthHz() { return bandwidthHz; }
    public int address() { return address; }
    public boolean promiscuous() { return promiscuous; }
    public long timingAdvanceUs() { return taUs; }
    /** The power setting, dBm, or NaN for the profile's default for the role. */
    public double requestedPowerDbm() { return powerDbm; }

    /** Switch the radio on (it hears and may transmit) or off (queues, HARQ and beacons cleared). */
    public synchronized void setActive(boolean on) {
        if (active == on) return;
        active = on;
        if (!on) reset();
    }

    /** A retune: frames heard on the old channel are gone (as in a real receiver, the queue is flushed). */
    private void flushReceive() {
        inbox.clear();
        held.clear();
        records.removeIf(r -> (r[2] & 0xff) == FramedRecords.RX);
        taUs = 0;
    }

    private void reset() {
        inbox.clear();
        held.clear();
        records.clear();
        harq.clear();
        peers.clear();
        lastSeq.clear();
        java.util.Arrays.fill(beacons, null);
        taUs = 0;
    }

    /** Select a PHY profile: channel 1 at the full channel bandwidth, the role's default power. Clears peers and beacons. */
    public synchronized void setProfile(PhyProfile p) {
        if (p == null) throw new IllegalArgumentException("unknown PHY profile (use " + profileNames() + ")");
        profile = p;
        channel = 1;
        bandwidthHz = p.bandwidthHz;
        powerDbm = Double.NaN;
        harq.clear();
        peers.clear();
        java.util.Arrays.fill(beacons, null);
        flushReceive();
    }

    public synchronized void setRole(Role r) {
        if (r == null) throw new IllegalArgumentException("role must be bs or ue");
        role = r;
        flushReceive();
    }

    public synchronized void setChannel(int n) {
        if (!profile.validChannel(n)) throw new IllegalArgumentException(profile.displayName + " channel " + n + " (1-" + profile.channels + ")");
        channel = n;
        flushReceive();
    }

    public synchronized void setBandwidth(double hz) {
        if (!profile.validBandwidth(hz)) throw new IllegalArgumentException(profile.displayName + " bandwidth " + fmtMhz(hz)
                + " MHz (use " + bandwidthList(profile) + " MHz)");
        bandwidthHz = profile.bandwidths()[indexOfBandwidth(hz)];
        flushReceive();
    }

    /** Requested transmit power, dBm (capped at the hardware's {@link Host#maxTxDbm} when sending). */
    public void setPower(double dbm) {
        if (!Double.isFinite(dbm) || dbm < -60 || dbm > 70) throw new IllegalArgumentException("power must be -60..70 dBm, got " + dbm);
        powerDbm = dbm;
    }

    public void setAddress(int a) {
        if (a == 0 || a == BROADCAST) throw new IllegalArgumentException("address must not be 0 or ffffffff");
        address = a;
    }

    public void setPromiscuous(boolean on) {
        promiscuous = on;
    }

    /** Power a frame is sent at now, dBm: the request (or the profile's default for the role) capped by the hardware. */
    public double txPowerDbm() {
        double want = Double.isNaN(powerDbm) ? (role == Role.BS ? profile.towerTxDbm : profile.ueTxDbm) : powerDbm;
        return Math.min(want, host.maxTxDbm());
    }

    /** Channel this radio transmits on: a base station's downlink, a UE's uplink. */
    public Channel txChannel() {
        return role == Role.BS ? profile.downlink(channel, bandwidthHz) : profile.uplink(channel, bandwidthHz);
    }

    /** Channel this radio hears: a base station's uplink, a UE's downlink. */
    public Channel rxChannel() {
        return role == Role.BS ? profile.uplink(channel, bandwidthHz) : profile.downlink(channel, bandwidthHz);
    }

    /** Weakest frame worth decoding: the noise floor plus CQI 1's threshold, less 1 dB, dBm. */
    public double sensitivityDbm() {
        return CqiTable.noiseDbm(rxChannel().bandwidthHz(), host.noiseFigureDb()) + CqiTable.requiredSinrDb(CqiTable.ROBUST_CQI) - 1;
    }

    /** The current slot index on the airtime clock. */
    public long slot() {
        RadioMedium m = host.medium();
        return m == null ? 0 : Math.floorDiv(m.nowMicros(), profile.slotUs);
    }

    /**
     * One control command (the {@code /dev/sdrctl} / {@code /dev/framesctl} framed lines):
     * {@code phy lte|mmwave}, {@code role bs|ue}, {@code channel N}, {@code phy_bw HZ},
     * {@code power DBM}, {@code address HEX}, {@code promisc 0|1}, and the wake filter
     * {@code wake tmsi N} / {@code wake off}. Returns false if the command isn't a framed one.
     */
    public boolean control(String[] p) {
        if (p.length == 0) return false;
        String arg = p.length > 1 ? p[1] : null;
        switch (p[0]) {
            case "wake" -> {
                // the modem's wake on page (docs/TELEPHONY.md): while the phone sleeps its
                // computer is stopped; the radio stays on the air and watches the paging frames
                if ("off".equals(arg)) wakeTmsi = 0;
                else if ("tmsi".equals(arg) && p.length > 2) wakeTmsi = (int) Long.parseLong(p[2]);
                else throw new IllegalArgumentException("wake tmsi N | wake off");
            }
            case "phy" -> setProfile(PhyProfile.byId(need(p)));
            case "role" -> setRole(parseRole(need(p)));
            case "channel" -> setChannel(Integer.parseInt(need(p)));
            case "phy_bw" -> setBandwidth(parseHz(need(p)));
            case "power" -> setPower(Double.parseDouble(need(p)));
            case "address" -> setAddress(parseAddress(need(p)));
            case "promisc" -> setPromiscuous("1".equals(arg) || "on".equals(arg) || "true".equals(arg));
            default -> { return false; }
        }
        return true;
    }

    private static String need(String[] p) {
        if (p.length < 2) throw new IllegalArgumentException(p[0] + " needs a value");
        return p[1];
    }

    public static Role parseRole(String s) {
        if (s == null) throw new IllegalArgumentException("role must be bs or ue");
        return switch (s.trim().toLowerCase(Locale.ROOT)) {
            case "bs", "site", "enb", "gnb", "base" -> Role.BS;
            case "ue", "device", "phone" -> Role.UE;
            default -> throw new IllegalArgumentException("role must be bs or ue, got " + s);
        };
    }

    public static int parseAddress(String s) {
        String t = s.trim().toLowerCase(Locale.ROOT);
        if (t.startsWith("0x")) t = t.substring(2);
        return (int) Long.parseLong(t, 16);
    }

    static double parseHz(String s) {
        String t = s.trim().toLowerCase(Locale.ROOT);
        double mul = 1;
        if (t.endsWith("m")) { mul = 1e6; t = t.substring(0, t.length() - 1); }
        else if (t.endsWith("k")) { mul = 1e3; t = t.substring(0, t.length() - 1); }
        else if (t.endsWith("g")) { mul = 1e9; t = t.substring(0, t.length() - 1); }
        return Double.parseDouble(t) * mul;
    }

    private int indexOfBandwidth(double hz) {
        double[] b = profile.bandwidths();
        for (int i = 0; i < b.length; i++) if (Math.abs(b[i] - hz) < 1e3) return i;
        return b.length - 1;
    }

    static String profileNames() {
        StringBuilder sb = new StringBuilder();
        for (PhyProfile p : PhyProfile.values()) sb.append(sb.length() > 0 ? ", " : "").append(p.id);
        return sb.toString();
    }

    static String bandwidthList(PhyProfile p) {
        StringBuilder sb = new StringBuilder();
        for (double b : p.bandwidths()) sb.append(sb.length() > 0 ? ", " : "").append(fmtMhz(b));
        return sb.toString();
    }

    static String fmtMhz(double hz) {
        double m = hz / 1e6;
        return m == Math.rint(m) ? String.valueOf((long) m) : String.format(Locale.ROOT, "%.1f", m);
    }

    // ------------------------------------------------------------ status

    /** {@code key value} lines (appended to the SDR's ctl file in framed mode, or a modem's framesctl file). */
    public synchronized String describe() {
        StringBuilder sb = new StringBuilder();
        for (Map.Entry<String, Object> e : status().entrySet()) {
            Object v = e.getValue();
            if (v instanceof Boolean b) v = b ? 1 : 0;
            else if (v instanceof Double d) v = String.format(Locale.ROOT, "%.1f", d);
            sb.append(e.getKey()).append(' ').append(v).append('\n');
        }
        return sb.toString();
    }

    /** Settings, the channel plan entry and counters. */
    public synchronized Map<String, Object> status() {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("framed", active);
        m.put("phy", profile.id);
        m.put("role", role.name().toLowerCase(Locale.ROOT));
        m.put("channel", channel);
        m.put("phy_bw", bandwidthHz);
        m.put("tx_freq", txChannel().centerHz());
        m.put("rx_freq", rxChannel().centerHz());
        m.put("power_dbm", txPowerDbm());
        m.put("max_power_dbm", host.maxTxDbm());
        m.put("address", String.format("%08x", address));
        m.put("promisc", promiscuous);
        m.put("slot_us", profile.slotUs);
        m.put("slot", slot());
        m.put("ta_us", taUs);
        m.put("tx_frames", txFrames);
        m.put("tx_bytes", txBytes);
        m.put("rx_frames", rxFrames);
        m.put("rx_bytes", rxBytes);
        m.put("rx_collisions", rxCollisions);
        m.put("rx_dropped", rxDropped);
        m.put("harq_acked", harqAcked);
        m.put("harq_retx", harqRetx);
        m.put("harq_failed", harqFailed);
        m.put("beacons_sent", beaconsSent);
        m.put("wake_tmsi", wakeTmsi == 0 ? "off" : Integer.toUnsignedString(wakeTmsi));
        m.put("page_wakes", pageWakes);
        m.put("blocked", blocked);
        int active = 0;
        for (Beacon b : beacons) if (b != null) active++;
        m.put("beacons", active);
        return m;
    }

    /** What the radio knows of each peer it heard: {address: {rssi_dbm, sinr_db, cqi, reported_cqi, age_ms, delay_us, frames}}. */
    public synchronized Map<String, Object> peers() {
        Map<String, Object> out = new LinkedHashMap<>();
        RadioMedium m = host.medium();
        long now = m == null ? 0 : m.nowMicros();
        for (Map.Entry<Integer, Peer> e : peers.entrySet()) {
            Peer p = e.getValue();
            Map<String, Object> v = new LinkedHashMap<>();
            v.put("rssi_dbm", Math.round(p.rssiDbm * 10) / 10.0);
            v.put("sinr_db", Math.round(p.sinrDb * 10) / 10.0);
            v.put("cqi", p.cqi);
            v.put("reported_cqi", p.reported);
            v.put("age_ms", Math.max(0, (now - p.seenUs) / 1000));
            v.put("delay_us", p.delayUs);
            v.put("frames", p.frames);
            out.put(String.format("%08x", e.getKey()), v);
        }
        return out;
    }

    /** The PHY profiles: id, name, channels with frequencies, bandwidths, slot, power and CQI range. */
    public static List<Map<String, Object>> profiles() {
        List<Map<String, Object>> out = new ArrayList<>();
        for (PhyProfile p : PhyProfile.values()) {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("id", p.id);
            m.put("name", p.displayName);
            m.put("channels", p.channels);
            List<Map<String, Object>> plan = new ArrayList<>();
            for (int n = 1; n <= p.channels; n++) {
                Map<String, Object> c = new LinkedHashMap<>();
                c.put("channel", n);
                c.put("downlink_hz", p.downlink(n).centerHz());
                c.put("uplink_hz", p.uplink(n).centerHz());
                plan.add(c);
            }
            m.put("plan", plan);
            List<Double> bws = new ArrayList<>();
            for (double b : p.bandwidths()) bws.add(b);
            m.put("bandwidths", bws);
            m.put("subchannels", p.ulSlices);
            m.put("slot_us", p.slotUs);
            m.put("bs_power_dbm", p.towerTxDbm);
            m.put("ue_power_dbm", p.ueTxDbm);
            m.put("max_cqi", p.maxCqi);
            out.add(m);
        }
        return out;
    }

    // ------------------------------------------------------------ device-file side

    public void addWaiter(Object w) { waiters.add(w); }
    public void removeWaiter(Object w) { waiters.remove(w); }

    /** True once a record waits (after a pump). Doesn't notify waiters: pollers call it holding their own waiter's lock. */
    public boolean readReady() {
        pump(false);
        synchronized (this) {
            return !records.isEmpty();
        }
    }

    public synchronized int available() {
        int n = 0;
        for (byte[] r : records) n += r.length;
        return n;
    }

    /** Take whole records that fit in {@code cap} bytes; null if none waits. Throws if the next record doesn't fit. */
    public synchronized byte[] take(int cap) {
        if (records.isEmpty()) return null;
        if (records.peekFirst().length > cap)
            throw new IllegalArgumentException("buffer of " + cap + " bytes is smaller than the next record (" + records.peekFirst().length + "); read with " + FramedRecords.MAX_RECORD);
        java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
        while (!records.isEmpty() && out.size() + records.peekFirst().length <= cap) out.writeBytes(records.pollFirst());
        return out.toByteArray();
    }

    /** Apply the records a program wrote. Whole records only. */
    public void write(byte[] data, int off, int len) {
        if (!active) throw new IllegalStateException("framed mode is off");
        int end = off + len, p = off;
        while (p < end) {
            int n = FramedRecords.recordLength(data, p, end);
            if (n < 3 || p + n > end) throw new IllegalArgumentException("partial record (write whole records; each starts with its u16 length)");
            int type = data[p + 2] & 0xff;
            switch (type) {
                case FramedRecords.TX -> send(FramedRecords.parseTx(data, p, n));
                case FramedRecords.BEACON -> beacon(FramedRecords.parseBeacon(data, p, n));
                default -> throw new IllegalArgumentException("unknown record type 0x" + Integer.toHexString(type));
            }
            p += n;
        }
    }

    // ------------------------------------------------------------ transmit

    /** Queue a frame (a TX record). Problems are reported as a rejected STATUS record, not thrown. */
    public void send(FramedRecords.TxRequest t) {
        Emission e;
        Harq h = null;
        String problem = null;
        long slot = 0;
        int cqi = 0;
        synchronized (this) {
            RadioMedium m = host.medium();
            long now = m == null ? 0 : m.nowMicros();
            if (!active || m == null) problem = "off";
            else if (t.payload().length > MAX_PAYLOAD) problem = "too big";
            else if (t.subchannel() > profile.ulSlices) problem = "subchannel";
            if (problem == null) {
                long next = Math.floorDiv(now + taUs, profile.slotUs) + 1;
                slot = t.slot() == 0 ? next : t.slot();
                if (slot * profile.slotUs - (role == Role.UE ? taUs : 0) < now) problem = "late";
                else if (slot * profile.slotUs - now > MAX_AHEAD_US) problem = "too far ahead";
            }
            if (problem != null) {
                push(FramedRecords.status(FramedRecords.RESULT_REJECTED, t.dst(), t.tag(), 0, 0, slot, 0));
                return;
            }
            cqi = chooseCqi(t.dst(), t.cqi());
            int s = nextSeq();
            boolean wantAck = (t.flags() & FramedRecords.FLAG_HARQ) != 0 && t.dst() != BROADCAST;
            int flags = wantAck ? FramedRecords.FLAG_HARQ : 0;
            e = emission(flags, t.dst(), s, t.payload(), slot, cqi, t.subchannel());
            if (wantAck) {
                h = new Harq(s, t.dst(), t.tag(), t.subchannel(), t.payload());
                h.attempts = 1;
                h.cqi = cqi;
                h.slot = slot;
                h.airUs = e.durationMicros();
                h.deadlineUs = slot * profile.slotUs + (long) PhyProfile.HARQ_RETX_SLOTS * profile.slotUs;
                harq.add(h);
            }
        }
        boolean ok = transmit(e);
        synchronized (this) {
            if (!ok) {
                if (h != null) harq.remove(h);
                push(FramedRecords.status(FramedRecords.RESULT_REJECTED, t.dst(), t.tag(), 1, cqi, slot, 0));
            } else if (h == null) {
                push(FramedRecords.status(FramedRecords.RESULT_SENT, t.dst(), t.tag(), 1, cqi, slot, e.durationMicros()));
            }
        }
        notifyWaiters();
    }

    /** Start, replace or stop (period 0) a beacon. */
    public void beacon(FramedRecords.BeaconRequest b) {
        synchronized (this) {
            if (b.index() < 0 || b.index() >= BEACONS) throw new IllegalArgumentException("beacon index 0-" + (BEACONS - 1));
            if (b.periodMs() == 0) {
                beacons[b.index()] = null;
                return;
            }
            if (b.periodMs() < MIN_BEACON_MS || b.periodMs() > MAX_BEACON_MS)
                throw new IllegalArgumentException("beacon period " + MIN_BEACON_MS + "-" + MAX_BEACON_MS + " ms");
            if (b.payload().length > MAX_PAYLOAD) throw new IllegalArgumentException("beacon payload over " + MAX_PAYLOAD + " bytes");
            if (b.subchannel() > profile.ulSlices) throw new IllegalArgumentException("subchannel 0-" + profile.ulSlices);
            Beacon x = new Beacon();
            x.payload = b.payload().clone();
            x.periodUs = b.periodMs() * 1000;
            x.cqi = b.cqi() == 0 ? CqiTable.ROBUST_CQI : Math.min(profile.maxCqi, b.cqi());
            x.sub = b.subchannel();
            RadioMedium m = host.medium();
            long now = m == null ? 0 : m.nowMicros();
            x.nextUs = (Math.floorDiv(now + profile.slotUs, x.periodUs) + 1) * x.periodUs;   // aligned to the airtime clock
            beacons[b.index()] = x;
        }
        pump();
    }

    private int nextSeq() {
        seq = (seq + 1) & 0xffff;
        return seq;
    }

    private int chooseCqi(int dst, int requested) {
        if (requested > 0) return Math.min(profile.maxCqi, requested);
        if (dst == BROADCAST) return CqiTable.ROBUST_CQI;
        Peer p = peers.get(dst);
        return p == null || p.reported <= 0 ? CqiTable.ROBUST_CQI : Math.min(profile.maxCqi, p.reported);
    }

    /** The emission for one frame (lock held). */
    private Emission emission(int flags, int dst, int s, byte[] payload, long slot, int cqi, int sub) {
        byte[] air = new byte[AIR_HEADER + payload.length];
        Peer p = dst == BROADCAST ? null : peers.get(dst);
        ByteBuffer.wrap(air).order(ByteOrder.LITTLE_ENDIAN).put((byte) MAGIC).put((byte) VERSION).put((byte) flags)
                .put((byte) (p == null ? 0 : p.cqi)).putInt(address).putInt(dst).putShort((short) s).putShort((short) 0).put(payload);
        Channel ch = txChannel();
        if (sub > 0) ch = profile.slice(ch, sub - 1);
        double rate = CqiTable.rateBps(profile, cqi, ch.bandwidthHz());
        long air_us = CqiTable.airtimeUs(profile, air.length, rate);
        long start = slot * profile.slotUs - (role == Role.UE ? taUs : 0);
        return Emission.frame(ch, txPowerDbm(), start, air_us, CqiTable.modulation(cqi), rate, air);
    }

    /** Put an emission on the air (lock NOT held: the medium calls receivers synchronously). */
    private boolean transmit(Emission e) {
        RadioMedium m = host.medium();
        if (m == null) return false;
        Emission out = host.beforeTransmit(e);
        if (out == null) {
            synchronized (this) { blocked++; }
            return false;
        }
        m.transmit(host.endpoint(), out);
        synchronized (this) {
            txFrames++;
            txBytes += e.payload().length - AIR_HEADER;
        }
        return true;
    }

    // ------------------------------------------------------------ receive

    /** The medium decoded a frame at this radio's endpoint (any thread). */
    public void onReceive(Reception r) {
        if (!active) return;
        byte[] air = r.payload();
        if (air == null || air.length < AIR_HEADER || (air[0] & 0xff) != MAGIC) return;
        if (!r.emission().channel().overlaps(rxChannel())) return;   // sent to the channel this radio just left
        Set<Emission> known = Collections.newSetFromMap(new IdentityHashMap<>());
        RadioMedium m = host.medium();
        Emission e = r.emission();
        if (m != null) m.forEachHeard(host.endpoint(), rxChannel(), e.startMicros(), e.endMicros(), h -> known.add(h.emission()));
        inbox.add(new Held(r, air, known));
    }

    /** Run the radio's timers: release received frames that have ended, HARQ, beacons. Any thread. */
    public void pump() {
        pump(true);
    }

    private void pump(boolean notify) {
        if (!active) return;
        RadioMedium m = host.medium();
        if (m == null) return;
        List<Emission> out = new ArrayList<>();
        boolean ready;
        synchronized (this) {
            long now = m.nowMicros();
            for (Held h; (h = inbox.poll()) != null; ) held.add(h);
            held.sort((a, b) -> Long.compare(a.r.timestampMicros(), b.r.timestampMicros()));
            for (Iterator<Held> it = held.iterator(); it.hasNext(); ) {
                Held h = it.next();
                if (h.r.timestampMicros() > now) break;
                it.remove();
                release(m, h, now, out);
            }
            for (Iterator<Harq> it = harq.iterator(); it.hasNext(); ) {
                Harq h = it.next();
                if (h.acked) {
                    it.remove();
                    harqAcked++;
                    push(FramedRecords.status(FramedRecords.RESULT_ACKED, h.dst, h.tag, h.attempts, h.cqi, h.slot, h.airUs));
                } else if (now >= h.deadlineUs) {
                    if (h.attempts > PhyProfile.HARQ_MAX_RETX) {
                        it.remove();
                        harqFailed++;
                        Peer p = peers.get(h.dst);
                        if (p != null) peers.put(h.dst, new Peer(p.rssiDbm, p.sinrDb, p.cqi, Math.max(1, p.reported - 2), p.seenUs, p.delayUs, p.frames));
                        push(FramedRecords.status(FramedRecords.RESULT_FAILED, h.dst, h.tag, h.attempts, h.cqi, h.slot, h.airUs));
                    } else {
                        h.attempts++;
                        harqRetx++;
                        h.cqi = Math.max(CqiTable.ROBUST_CQI, h.cqi - 2);
                        h.slot = Math.floorDiv(now + taUs, profile.slotUs) + 1;
                        Emission e = emission(FramedRecords.FLAG_HARQ | FramedRecords.FLAG_RETX, h.dst, h.seq, h.payload, h.slot, h.cqi, h.sub);
                        h.airUs = e.durationMicros();
                        h.deadlineUs = h.slot * profile.slotUs + (long) PhyProfile.HARQ_RETX_SLOTS * profile.slotUs;
                        out.add(e);
                    }
                }
            }
            for (Beacon b : beacons) {
                if (b == null) continue;
                if (b.nextUs < now) b.nextUs = (Math.floorDiv(now, b.periodUs) + 1) * b.periodUs;   // a late pump skips missed beacons
                while (b.nextUs <= now + LEAD_US) {
                    b.seq = (b.seq + 1) & 0xffff;
                    long slot = Math.floorDiv(b.nextUs, profile.slotUs);
                    out.add(emission(FramedRecords.FLAG_BEACON, BROADCAST, b.seq, b.payload, slot, b.cqi, b.sub));
                    beaconsSent++;
                    b.nextUs += b.periodUs;
                }
            }
            ready = !records.isEmpty();
        }
        for (Emission e : out) transmit(e);
        if (ready && notify) notifyWaiters();
    }

    /** One received frame has ended: collision re-check, measurements, ACK, delivery (lock held). */
    private void release(RadioMedium m, Held h, long now, List<Emission> out) {
        Reception r = h.r;
        Emission e = r.emission();
        double sinr = r.sinrDb();
        // Emissions that overlapped this frame but weren't on the air when it was delivered.
        Channel rx = rxChannel();
        Channel within = BasicRadioMedium.intersection(e.channel(), rx);
        double[] extra = {0};
        m.forEachHeard(host.endpoint(), within, e.startMicros(), e.endMicros(), x -> {
            if (x.emission() == e || h.known.contains(x.emission()) || x.from().id().equals(r.from())) return;
            double w = BasicRadioMedium.overlapFraction(x.emission().channel(), within);
            if (w > 0) extra[0] += w * BasicRadioMedium.dbmToMw(x.rxPowerDbm());
        });
        if (extra[0] > 0) {
            double before = BasicRadioMedium.dbmToMw(r.rssiDbm() - sinr);
            double sinrNow = r.rssiDbm() - BasicRadioMedium.mwToDbm(before + extra[0]);
            int bits = e.payload().length * 8;
            double per0 = BasicRadioMedium.packetErrorRate(e.modulation(), sinr, bits);
            double per1 = BasicRadioMedium.packetErrorRate(e.modulation(), sinrNow, bits);
            double lose = per1 >= 1 ? 1 : Math.max(0, (per1 - per0) / Math.max(1e-12, 1 - per0));
            sinr = sinrNow;
            if (roll(e) < lose) {
                rxCollisions++;
                return;
            }
        }
        ByteBuffer b = ByteBuffer.wrap(h.air).order(ByteOrder.LITTLE_ENDIAN);
        int flags = b.get(2) & 0xff, report = b.get(3) & 0xff, src = b.getInt(4), dst = b.getInt(8), s = b.getShort(12) & 0xffff;
        boolean forMe = dst == address || dst == BROADCAST;
        int cqi = CqiTable.cqiFor(profile, sinr);
        long delay = Math.max(0, r.timestampMicros() - e.endMicros());
        Peer old = peers.get(src);
        int reported = dst == address && report > 0 ? report : old == null ? 0 : old.reported;
        peers.put(src, new Peer(r.rssiDbm(), sinr, cqi, reported, now, delay, old == null ? 1 : old.frames + 1));
        if (role == Role.UE) taUs = delay;
        if ((flags & FramedRecords.FLAG_ACK) != 0) {
            if (dst == address) for (Harq x : harq) if (x.dst == src && x.seq == s) x.acked = true;
            return;   // ACKs are the radio's own business
        }
        if (dst == address && (flags & FramedRecords.FLAG_HARQ) != 0) {
            long slot = Math.max(Math.floorDiv(e.startMicros(), profile.slotUs) + PhyProfile.HARQ_ACK_SLOTS,
                    Math.floorDiv(now + taUs, profile.slotUs) + 1);
            out.add(emission(FramedRecords.FLAG_ACK, src, s, new byte[0], slot, CqiTable.ROBUST_CQI, 0));
            Integer last = lastSeq.put(src, s);
            if ((flags & FramedRecords.FLAG_RETX) != 0 && last != null && last == s) return;   // a duplicate: ACK again, deliver once
        }
        if (!forMe && !promiscuous) {
            filtered++;
            return;
        }
        byte[] payload = java.util.Arrays.copyOfRange(h.air, AIR_HEADER, h.air.length);
        if (wakeTmsi != 0 && dst == BROADCAST && pagesTmsi(payload, wakeTmsi)) {
            pageWakes++;
            host.page();
        }
        int sub = subchannelOf(e.channel(), rx);
        push(FramedRecords.rx(flags, src, dst, s, cqi, CqiTable.cqiOf(e.modulation()), r.rssiDbm(), sinr, r.timestampMicros(),
                Math.floorDiv(e.startMicros() + delay, profile.slotUs), sub, dst == address ? report : 0, delay, payload));
        rxFrames++;
        rxBytes += payload.length;
    }

    /** 0 if {@code ch} fills {@code whole}, else the 1-based quarter it sits in. */
    private int subchannelOf(Channel ch, Channel whole) {
        if (ch.bandwidthHz() >= whole.bandwidthHz() * 0.9) return 0;
        int k = (int) Math.floor((ch.centerHz() - whole.lowHz()) / (whole.bandwidthHz() / profile.ulSlices));
        return Math.max(0, Math.min(profile.ulSlices - 1, k)) + 1;
    }

    private double roll(Emission e) {
        long h = 0x5DEECE66DL ^ e.startMicros() * 0x9E3779B97F4A7C15L ^ address * 0xC2B2AE3D27D4EB4FL;
        if (e.payload() != null) h ^= java.util.Arrays.hashCode(e.payload());
        h ^= h >>> 33; h *= 0xff51afd7ed558ccdL; h ^= h >>> 33; h *= 0xc4ceb9fe1a85ec53L; h ^= h >>> 33;
        return (h >>> 11) * 0x1.0p-53;
    }

    /** The TMSI the wake filter watches (0: off). */
    private volatile int wakeTmsi;
    /** Pages that woke the device. */
    private volatile long pageWakes;

    public int wakeTmsi() {
        return wakeTmsi;
    }

    public long pageWakes() {
        return pageWakes;
    }

    /**
     * Is {@code payload} an ecm-cell Paging message ({@code 0x09}, a count, then
     * big-endian 32-bit TMSIs: rust/crates/ecm-cell/src/air.rs) naming {@code tmsi}?
     */
    static boolean pagesTmsi(byte[] payload, int tmsi) {
        if (payload.length < 2 || (payload[0] & 0xff) != 0x09) return false;
        int n = payload[1] & 0xff;
        if (payload.length != 2 + 4 * n) return false;
        for (int i = 0; i < n; i++) {
            int o = 2 + 4 * i;
            int t = (payload[o] & 0xff) << 24 | (payload[o + 1] & 0xff) << 16 | (payload[o + 2] & 0xff) << 8 | payload[o + 3] & 0xff;
            if (t == tmsi) return true;
        }
        return false;
    }

    private void push(byte[] record) {
        boolean wasEmpty = records.isEmpty();
        if (records.size() >= MAX_QUEUE) {
            records.pollFirst();
            rxDropped++;
        }
        records.addLast(record);
        if (wasEmpty) host.event("framed_rx");
    }

    private void notifyWaiters() {
        for (Object w : waiters) {
            synchronized (w) {
                w.notifyAll();
            }
        }
    }
}
