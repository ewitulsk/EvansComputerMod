package com.example.evanscomputermod.radio.cellular;

import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;

import java.security.SecureRandom;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.Supplier;

/**
 * A cellular modem (the UE side), free of Minecraft: one {@link RadioEndpoint}
 * that searches for its SIM's carrier, attaches, carries Ethernet frames for the
 * computer's kernel ({@code wwan0}) and hands over between towers.
 *
 * <p><b>Idle.</b> With a SIM it scans the downlink channels of its plan, dwelling
 * {@link #DWELL_US} on each and recording every tower beacon it hears (carrier,
 * cell key, power, RSSI). As soon as a tower of the SIM's carrier is heard it
 * sends an attach request on that tower's random-access slice; towers of other
 * carriers are listed by {@link #cells()} but never asked.
 *
 * <p><b>Connected.</b> It stays on the serving channel, reports its downlink SINR
 * in every uplink frame (and in a keep-alive each second when idle), picks its
 * uplink CQI from the beacon by reciprocity, and keeps measuring the beacons of
 * neighbouring towers of the same carrier. When one has been {@link #HYSTERESIS_DB}
 * stronger than the serving tower for {@link #TIME_TO_TRIGGER_US}, it attaches to
 * that tower while still connected to the old one, and switches only when the new
 * tower accepts (make before break, like an LTE X2 handover). Losing the serving
 * tower's beacons for {@link #RADIO_LINK_FAILURE_US} drops back to idle search
 * (re-establishment on whichever tower is best). Its MAC never changes, so the
 * kernel's addresses, ARP entries and TCP connections survive either path.
 *
 * <p>Threads: the medium calls {@code onReceive} from any thread (user data is
 * queued for the kernel at once; control frames wait for {@link #tick}); the
 * kernel calls {@link #sendEthernet}/{@link #pollEthernet} from the computer's
 * worker thread; {@link #tick} and configuration run on the owner's thread.
 */
public final class ModemLink implements RadioEndpoint {

    public enum State {
        /** No SIM card. */
        NO_SIM,
        /** Scanning for a tower of the SIM's carrier. */
        SEARCHING,
        /** Asked a tower to attach, waiting for its answer. */
        ATTACHING,
        /** Attached: carrier up on wwan0. */
        CONNECTED,
        /** A tower refused the SIM (wrong key, unknown, quota...); retries later. */
        REFUSED,
        /** Switched off (module unloaded). */
        OFF
    }

    /** One tower as heard from beacons. */
    public record Cell(int cellKey, UUID towerId, String carrier, CellTech tech, int channel, double txPowerDbm, int load,
                       double rssiDbm, double sinrDb, long lastHeardUs) {}

    static final long DWELL_US = 300_000;
    static final long ATTACH_TIMEOUT_US = 400_000;
    static final int ATTACH_TRIES = 3;
    static final long RADIO_LINK_FAILURE_US = 1_000_000;
    static final long FRESH_US = 1_500_000;
    static final long KEEPALIVE_US = 1_000_000;
    static final long TIME_TO_TRIGGER_US = 320_000;
    static final double HYSTERESIS_DB = 3;
    static final long MAX_BACKLOG_US = 250_000;
    static final int RX_LIMIT = 256;

    private final UUID id;
    private final CellTech tech;
    private final byte[] mac;
    private final Supplier<RadioMedium> medium;
    private final AntennaPattern antenna;
    private final SecureRandom rng = new SecureRandom();

    private volatile SimCard sim;
    private volatile Pose pose;
    private volatile double speedMps;
    private volatile State state = State.NO_SIM;
    private volatile String message = "No SIM card";
    private RadioMedium registeredWith;

    private volatile int scanChannel = 1;
    private long dwellStartUs = Long.MIN_VALUE;
    private final Map<Integer, Cell> cells = new ConcurrentHashMap<>();
    private final Map<Integer, Long> barredUntil = new ConcurrentHashMap<>();

    // Serving cell (volatile: read by the kernel's thread and the medium's).
    private volatile Cell serving;
    private volatile int rnti = -1, slice;
    private volatile double dlSinrDb = Double.NaN;
    private volatile long connectedSinceUs;

    // Attach in progress (initial or handover).
    private Cell pending;
    private boolean pendingHandover;
    private long pendingNonce, pendingSentUs;
    private int pendingTries;
    private int handoverCandidate;
    private long handoverSinceUs;

    private final ConcurrentLinkedQueue<CellFrame.Frame> control = new ConcurrentLinkedQueue<>();
    private final ConcurrentLinkedQueue<byte[]> rx = new ConcurrentLinkedQueue<>();
    private final AtomicInteger rxQueued = new AtomicInteger();
    private volatile Runnable rxNotify;
    private long nextFreeUs, lastUplinkUs;

    private final AtomicLong txFrames = new AtomicLong(), rxFrames = new AtomicLong(), txBytes = new AtomicLong(),
            rxBytes = new AtomicLong(), txDropped = new AtomicLong(), rxDropped = new AtomicLong(), handovers = new AtomicLong(),
            attaches = new AtomicLong();
    private final List<String> events = new ArrayList<>();

    public ModemLink(UUID id, CellTech tech, byte[] mac, Supplier<RadioMedium> medium) {
        this.id = id;
        this.tech = tech;
        this.mac = mac.clone();
        this.medium = medium;
        this.antenna = CellAntennas.modem(tech);
    }

    // ------------------------------------------------------------ configuration (owner thread)

    /** Insert (or remove, with null) the SIM. A different SIM detaches first. */
    public synchronized void setSim(SimCard s) {
        if (java.util.Objects.equals(s, sim)) return;
        if (state == State.CONNECTED) sendDetach();
        dropConnection(s == null ? "No SIM card" : "Searching for " + s.carrier());
        sim = s;
        state = s == null ? State.NO_SIM : State.SEARCHING;
        barredUntil.clear();
        invalidate();
    }

    public SimCard sim() {
        return sim;
    }

    /** Where the antenna is now; mobile hosts call this every tick. */
    public void setPose(Pose p) {
        Pose before = pose;
        pose = p;
        if (before != null && p != null && before.sameDimension(p)) speedMps = before.distanceTo(p) * 20;   // per tick
        RadioMedium m = registeredWith;
        if (m != null && before == null && p != null) m.register(this);
    }

    /** Called (from any thread) when Ethernet frames are waiting for the kernel. */
    public void setRxNotify(Runnable r) {
        rxNotify = r;
    }

    /** Switch the radio off: detach politely and leave the medium. */
    public synchronized void stop() {
        if (state == State.CONNECTED) sendDetach();
        dropConnection("Switched off");
        state = State.OFF;
        if (registeredWith != null) registeredWith.unregister(this);
        registeredWith = null;
    }

    /** Back on after {@link #stop}. */
    public synchronized void start() {
        if (state == State.OFF) state = sim == null ? State.NO_SIM : State.SEARCHING;
    }

    // ------------------------------------------------------------ status

    public CellTech tech() { return tech; }
    public byte[] mac() { return mac.clone(); }
    public State state() { return state; }
    public String message() { return message; }
    public boolean linkUp() { return state == State.CONNECTED; }
    public Cell serving() { return serving; }
    public int rnti() { return rnti; }
    public int slice() { return slice; }
    public double dlSinrDb() { return dlSinrDb; }
    public long txFrames() { return txFrames.get(); }
    public long rxFrames() { return rxFrames.get(); }
    public long txBytes() { return txBytes.get(); }
    public long rxBytes() { return rxBytes.get(); }
    public long handovers() { return handovers.get(); }
    public long attaches() { return attaches.get(); }
    public int scanChannel() { return scanChannel; }

    /** Every tower heard recently, strongest first. */
    public List<Cell> cells() {
        long now = now();
        List<Cell> out = new ArrayList<>();
        for (Cell c : cells.values()) if (now - c.lastHeardUs <= FRESH_US) out.add(c);
        out.sort(Comparator.comparingDouble(Cell::rssiDbm).reversed());
        return out;
    }

    public synchronized List<String> events() {
        return List.copyOf(events);
    }

    /** Downlink CQI the tower should be using, from our SINR. */
    public int downlinkCqi() {
        return CellPhy.cqiFor(tech, dlSinrDb);
    }

    /** The uplink CQI we pick now (reciprocity estimate from the serving beacon). */
    public int uplinkCqi() {
        Cell s = serving;
        return s == null ? CellPhy.ROBUST_CQI : CellPhy.cqiFor(tech, CellPhy.uplinkSinrEstimateDb(tech, s.rssiDbm, s.txPowerDbm));
    }

    /** Snapshot for the {@code status} peripheral method and the {@code cell} program. */
    public Map<String, Object> status() {
        Map<String, Object> m = new LinkedHashMap<>();
        SimCard s = sim;
        Cell c = serving;
        m.put("tech", tech.displayName);
        m.put("state", state.name().toLowerCase(java.util.Locale.ROOT));
        m.put("message", message);
        m.put("sim_carrier", s == null ? null : s.carrier());
        m.put("imsi", s == null ? null : s.imsiText());
        m.put("mac", TowerLink.macText(mac));
        m.put("carrier", c == null ? null : c.carrier);
        m.put("tower", c == null ? null : c.towerId.toString());
        m.put("channel", c == null ? scanChannel : c.channel);
        m.put("frequency_mhz", (c == null ? tech.downlink(scanChannel) : tech.downlink(c.channel)).centerHz() / 1e6);
        m.put("rssi_dbm", c == null ? null : round1(c.rssiDbm));
        m.put("sinr_db", Double.isNaN(dlSinrDb) || c == null ? null : round1(dlSinrDb));
        m.put("bars", c == null ? 0 : CellPhy.bars(dlSinrDb));
        m.put("dl_cqi", c == null ? null : downlinkCqi());
        m.put("dl_mbps", c == null ? null : round1(CellPhy.downlinkRateBps(tech, downlinkCqi()) / 1e6));
        m.put("ul_cqi", c == null ? null : uplinkCqi());
        m.put("ul_mbps", c == null ? null : round1(CellPhy.uplinkRateBps(tech, uplinkCqi()) / 1e6));
        m.put("rnti", c == null ? null : rnti);
        m.put("slice", c == null ? null : slice);
        m.put("tx_bytes", txBytes.get());
        m.put("rx_bytes", rxBytes.get());
        m.put("handovers", handovers.get());
        return m;
    }

    private static double round1(double v) {
        return Math.round(v * 10) / 10.0;
    }

    // ------------------------------------------------------------ kernel side (worker thread)

    /** Queue an Ethernet frame for the serving tower; false when not attached or the queue is full. */
    public boolean sendEthernet(byte[] eth) {
        Cell c = serving;
        int r = rnti;
        if (state != State.CONNECTED || c == null || r < 0 || eth.length < 14) {
            txDropped.incrementAndGet();
            return false;
        }
        byte[] f = CellFrame.dataUplink(c.cellKey, r, dlSinrDb, eth);
        if (!transmitUplink(f, uplinkCqi(), slice, c.channel)) {
            txDropped.incrementAndGet();
            return false;
        }
        txBytes.addAndGet(eth.length);
        return true;
    }

    /** The next Ethernet frame from the tower, or null. */
    public byte[] pollEthernet() {
        byte[] f = rx.poll();
        if (f != null) rxQueued.decrementAndGet();
        return f;
    }

    // ------------------------------------------------------------ tick (owner thread)

    public synchronized void tick() {
        RadioMedium m = medium.get();
        if (m != registeredWith) {
            if (registeredWith != null) registeredWith.unregister(this);
            registeredWith = m;
            if (m != null && pose != null && state != State.OFF) m.register(this);
        }
        if (m == null || state == State.OFF) return;
        long now = m.nowMicros();
        CellFrame.Frame f;
        while ((f = control.poll()) != null) onControl(f, now);
        SimCard s = sim;
        if (s == null) {
            state = State.NO_SIM;
            return;
        }
        switch (state) {
            case NO_SIM -> {
                state = State.SEARCHING;
                message = "Searching for " + s.carrier();
            }
            case SEARCHING, REFUSED -> search(s, now);
            case ATTACHING -> retryAttach(s, now);
            case CONNECTED -> connected(s, now);
            default -> { }
        }
    }

    private void search(SimCard s, long now) {
        if (dwellStartUs == Long.MIN_VALUE) dwellStartUs = now;
        if (now - dwellStartUs >= DWELL_US) {
            // End of a dwell: every beacon on this channel (and recently on others) is in; take the strongest.
            Cell best = bestOf(s, now, -1);
            if (best != null) {
                dwellStartUs = now;
                startAttach(s, best, false, now);
                return;
            }
            dwellStartUs = now;
            scanChannel = scanChannel % tech.channels + 1;
            invalidate();
        }
    }

    /** The strongest fresh, unbarred tower of the SIM's carrier (optionally only on {@code channel}). */
    private Cell bestOf(SimCard s, long now, int channel) {
        Cell best = null;
        for (Cell c : cells.values()) {
            if (!c.carrier.equals(s.carrier()) || now - c.lastHeardUs > FRESH_US) continue;
            if (channel > 0 && c.channel != channel) continue;
            Long bar = barredUntil.get(c.cellKey);
            if (bar != null && now < bar) continue;
            if (best == null || c.rssiDbm > best.rssiDbm) best = c;
        }
        return best;
    }

    private void startAttach(SimCard s, Cell target, boolean handover, long now) {
        pending = target;
        pendingHandover = handover;
        pendingNonce = rng.nextLong();
        pendingTries = 0;
        if (!handover) {
            state = State.ATTACHING;
            message = "Attaching to " + target.carrier + " (" + tech.displayName + " ch " + target.channel + ")";
            invalidate();
        }
        sendAttach(s, now);
    }

    private void sendAttach(SimCard s, long now) {
        Cell t = pending;
        pendingTries++;
        pendingSentUs = now;
        byte[] proof = SimCard.attachProof(s.key(), t.cellKey, s.imsi(), mac, pendingNonce);
        byte[] f = CellFrame.attach(t.cellKey, s.imsi(), mac, pendingNonce, t.sinrDb, pendingHandover ? CellFrame.FLAG_HANDOVER : 0, proof);
        transmitUplink(f, CellPhy.ROBUST_CQI, 0, t.channel);
    }

    private void retryAttach(SimCard s, long now) {
        if (pending == null) {
            state = State.SEARCHING;
            return;
        }
        if (now - pendingSentUs < ATTACH_TIMEOUT_US) return;
        if (pendingTries < ATTACH_TRIES) {
            sendAttach(s, now);
            return;
        }
        bar(pending.cellKey, now, 5_000_000);
        event("no answer from " + pending.carrier + " tower " + shortId(pending));
        boolean wasHandover = pendingHandover;
        pending = null;
        if (!wasHandover) {
            state = State.SEARCHING;
            message = "No answer from the tower; searching for " + s.carrier();
            invalidate();
        }
    }

    private void connected(SimCard s, long now) {
        Cell c = serving;
        if (c == null) {
            state = State.SEARCHING;
            return;
        }
        Cell fresh = cells.get(c.cellKey);
        if (fresh != null) serving = c = fresh;
        if (now - c.lastHeardUs > RADIO_LINK_FAILURE_US) {
            event("lost the signal of tower " + shortId(c));
            scanChannel = c.channel;
            dropConnection("Signal lost; searching for " + s.carrier());
            state = State.SEARCHING;
            dwellStartUs = now;
            invalidate();
            return;
        }
        if (pending != null && pendingHandover) {
            if (now - pendingSentUs >= ATTACH_TIMEOUT_US) {
                if (pendingTries < ATTACH_TRIES) sendAttach(s, now);
                else {
                    bar(pending.cellKey, now, 5_000_000);
                    pending = null;
                }
            }
        } else {
            // Handover: a neighbour of our carrier on the serving channel, clearly stronger for a while.
            Cell n = null;
            for (Cell x : cells.values()) {
                if (x.cellKey == c.cellKey || !x.carrier.equals(s.carrier()) || x.channel != c.channel || now - x.lastHeardUs > 500_000)
                    continue;
                Long bar = barredUntil.get(x.cellKey);
                if (bar != null && now < bar) continue;
                if (x.rssiDbm > c.rssiDbm + HYSTERESIS_DB && (n == null || x.rssiDbm > n.rssiDbm)) n = x;
            }
            if (n == null) {
                handoverCandidate = 0;
            } else if (n.cellKey != handoverCandidate) {
                handoverCandidate = n.cellKey;
                handoverSinceUs = now;
            } else if (now - handoverSinceUs >= TIME_TO_TRIGGER_US) {
                event(String.format(java.util.Locale.ROOT, "handover %s -> %s (%.1f vs %.1f dBm)", shortId(c), shortId(n), c.rssiDbm, n.rssiDbm));
                handoverCandidate = 0;
                startAttach(s, n, true, now);
            }
        }
        if (now - lastUplinkUs >= KEEPALIVE_US) transmitUplink(CellFrame.report(c.cellKey, rnti, dlSinrDb), CellPhy.ROBUST_CQI, slice, c.channel);
    }

    private void onControl(CellFrame.Frame f, long now) {
        SimCard s = sim;
        if (s == null) return;
        switch (f) {
            case CellFrame.Accept a -> {
                Cell t = pending;
                if (t == null || a.cellKey() != t.cellKey || a.imsi() != s.imsi()) return;
                if (!SimCard.proofEquals(a.proof(), SimCard.acceptProof(s.key(), t.cellKey, s.imsi(), pendingNonce, a.rnti()))) {
                    event("tower " + shortId(t) + " failed to prove it knows the SIM key (fake tower?)");
                    bar(t.cellKey, now, 30_000_000);
                    pending = null;
                    if (!pendingHandover) state = State.SEARCHING;
                    return;
                }
                boolean handover = pendingHandover && state == State.CONNECTED;
                pending = null;
                serving = cells.getOrDefault(t.cellKey, t);
                rnti = a.rnti();
                slice = a.slice();
                state = State.CONNECTED;
                connectedSinceUs = now;
                lastUplinkUs = now;
                if (handover) handovers.incrementAndGet();
                else attaches.incrementAndGet();
                message = "Connected to " + t.carrier + " (" + tech.displayName + " ch " + t.channel + ")";
                event((handover ? "handed over to " : "attached to ") + t.carrier + " tower " + shortId(t) + ", rnti " + rnti + ", slice " + slice);
                invalidate();
            }
            case CellFrame.Reject r -> {
                Cell t = pending;
                if (t == null || r.cellKey() != t.cellKey || r.imsi() != s.imsi()) return;
                boolean handover = pendingHandover && state == State.CONNECTED;
                pending = null;
                bar(t.cellKey, now, 10_000_000);
                event("tower " + shortId(t) + " refused the SIM: " + r.cause().text);
                if (!handover) {
                    state = State.REFUSED;
                    message = "Refused by " + t.carrier + ": " + r.cause().text;
                    invalidate();
                }
            }
            case CellFrame.Release rel -> {
                Cell c = serving;
                if (c == null || rel.cellKey() != c.cellKey || rel.rnti() != rnti) return;
                event("released by tower " + shortId(c) + (rel.cause() == CellFrame.Cause.NONE ? "" : ": " + rel.cause().text));
                if (rel.cause() == CellFrame.Cause.QUOTA || rel.cause() == CellFrame.Cause.REVOKED) {
                    bar(c.cellKey, now, 10_000_000);
                    dropConnection("Refused by " + c.carrier + ": " + rel.cause().text);
                    state = State.REFUSED;
                } else {
                    dropConnection("Released by the tower; searching for " + s.carrier());
                    state = State.SEARCHING;
                }
                invalidate();
            }
            default -> { }
        }
    }

    private void dropConnection(String why) {
        serving = null;
        rnti = -1;
        pending = null;
        handoverCandidate = 0;
        dlSinrDb = Double.NaN;
        message = why;
        rx.clear();
        rxQueued.set(0);
    }

    private void sendDetach() {
        Cell c = serving;
        if (c != null && rnti >= 0) transmitUplink(CellFrame.detach(c.cellKey, rnti), CellPhy.ROBUST_CQI, slice, c.channel);
    }

    private void bar(int cellKey, long now, long forUs) {
        barredUntil.put(cellKey, now + forUs);
    }

    private synchronized void event(String e) {
        events.add(e);
        if (events.size() > 16) events.remove(0);
    }

    private static String shortId(Cell c) {
        return String.format("%08x", c.cellKey);
    }

    // ------------------------------------------------------------ air

    private synchronized boolean transmitUplink(byte[] payload, int cqi, int sl, int channel) {
        RadioMedium m = registeredWith;
        if (m == null || pose == null) return false;
        long now = m.nowMicros();
        long start = Math.max(now, nextFreeUs);
        if (start - now > MAX_BACKLOG_US) return false;
        double rate = CellPhy.uplinkRateBps(tech, cqi);
        long dur = CellPhy.airtimeUs(tech, payload.length, rate);
        nextFreeUs = start + dur;
        lastUplinkUs = now;
        txFrames.incrementAndGet();
        m.transmit(this, Emission.frame(tech.uplinkSlice(channel, sl), tech.ueTxDbm, start, dur, CellPhy.modulation(cqi), rate, payload));
        return true;
    }

    private void invalidate() {
        RadioMedium m = registeredWith;
        if (m != null && pose != null) m.invalidate(this);
    }

    private long now() {
        RadioMedium m = medium.get();
        return m == null ? 0 : m.nowMicros();
    }

    @Override public UUID id() { return id; }
    @Override public Pose pose() { return pose; }
    @Override public AntennaPattern antenna() { return antenna; }
    @Override public double noiseFigureDb() { return tech.ueNoiseFigureDb; }
    @Override public double sensitivityDbm() { return CellPhy.ueSensitivityDbm(tech); }
    @Override public double maxTxPowerDbm() { return tech.ueTxDbm; }
    @Override public double speedMps() { return speedMps; }

    @Override
    public Channel tunedChannel() {
        State st = state;
        if (pose == null || st == State.OFF || st == State.NO_SIM) return null;
        Cell c = serving;
        if (st == State.CONNECTED && c != null) return tech.downlink(c.channel);
        Cell p = pending;
        if (st == State.ATTACHING && p != null) return tech.downlink(p.channel);
        return tech.downlink(scanChannel);
    }

    @Override
    public void onReceive(Reception r) {
        Emission e = r.emission();
        if (e.kind() != Emission.Kind.FRAME || e.payload() == null || CellPhy.cqiOf(e.modulation()) == 0) return;
        CellFrame.Frame f = CellFrame.decode(e.payload());
        if (f == null) return;
        long now = r.timestampMicros();
        switch (f) {
            case CellFrame.Beacon b -> {
                if (b.tech() != tech) return;
                Cell old = cells.get(b.cellKey());
                double rssi = old == null || now - old.lastHeardUs > FRESH_US ? r.rssiDbm() : 0.7 * old.rssiDbm + 0.3 * r.rssiDbm();
                double sinr = old == null || now - old.lastHeardUs > FRESH_US || Double.isNaN(old.sinrDb) ? r.sinrDb()
                        : 0.7 * old.sinrDb + 0.3 * r.sinrDb();
                Cell c = new Cell(b.cellKey(), b.towerId(), b.carrier(), b.tech(), b.channel(), b.txPowerDbm(), b.load(), rssi, sinr, now);
                cells.put(b.cellKey(), c);
                Cell s = serving;
                if (s != null && s.cellKey == b.cellKey()) measured(r.sinrDb());
            }
            case CellFrame.Data d when !d.uplink() -> {
                Cell s = serving;
                if (state != State.CONNECTED || s == null || d.cellKey() != s.cellKey) return;
                if (d.rnti() != rnti && d.rnti() != CellFrame.BROADCAST_RNTI) return;
                byte[] eth = d.ethernet();
                if (eth.length < 14) return;
                if (d.rnti() == CellFrame.BROADCAST_RNTI && java.util.Arrays.equals(java.util.Arrays.copyOfRange(eth, 6, 12), mac)) return;
                measured(r.sinrDb());
                if (rxQueued.incrementAndGet() > RX_LIMIT) {
                    rxQueued.decrementAndGet();
                    rxDropped.incrementAndGet();
                    return;
                }
                rxFrames.incrementAndGet();
                rxBytes.addAndGet(eth.length);
                rx.add(eth);
                Runnable n = rxNotify;
                if (n != null) n.run();
            }
            case CellFrame.Accept a -> control.add(a);
            case CellFrame.Reject j -> control.add(j);
            case CellFrame.Release rel -> control.add(rel);
            default -> { }
        }
    }

    private void measured(double sinr) {
        double old = dlSinrDb;
        dlSinrDb = Double.isNaN(old) ? sinr : 0.8 * old + 0.2 * sinr;
    }
}
