package com.example.evanscomputermod.radio.cellular;

import com.example.evanscomputermod.radio.framed.PhyProfile;
import com.example.evanscomputermod.radio.framed.CqiTable;
import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.Comparator;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.Supplier;

/**
 * One technology of a cell tower (an eNodeB / gNB), free of Minecraft: the radio
 * on the shared medium and a layer-2 bridge onto its backhaul cable segment, like
 * the Wi-Fi Access Point.
 *
 * <p><b>Air.</b> It beacons its carrier, cell key and transmit power every
 * {@link PhyProfile#beaconIntervalUs} on its downlink channel and hears devices on
 * the paired uplink, one {@link RadioEndpoint} per uplink slice so each slice has
 * its own (narrow) noise floor. Downlink frames go back to back on the airtime
 * clock at the CQI the device last reported, broadcasts at the worst device's.
 *
 * <p><b>Attach.</b> A device sends an attach request with its IMSI, MAC and a
 * proof over a nonce keyed by its SIM's secret; the {@link Core} (the carrier's
 * subscriber table) says whether that IMSI belongs to this tower's carrier, may
 * use data and what its key is. Accepted devices get a radio id (RNTI) and an
 * uplink slice, and the accept proves the carrier knows the key. Accepting a
 * device tells the core, which releases it from any other tower of the carrier
 * (handover: the device keeps its MAC, so the IP stack above never notices).
 *
 * <p><b>Bridge.</b> Uplink Ethernet frames go onto the cable through
 * {@link Backhaul} (the hub's {@code transmitFromPort}, which learns the device's
 * MAC behind this tower), or straight back down to another attached device.
 * Cable frames addressed to an attached device (or broadcast) go down the air.
 * Every byte either way is counted against the SIM's quota.
 *
 * <p>Threads: {@link #fromCable} (cable sender's thread) and the medium's
 * {@code onReceive} only queue; everything else happens in {@link #tick} on the
 * owner's (server) thread.
 */
public final class TowerLink {

    /** The carrier core network a tower asks about subscribers. */
    public interface Core {
        /** What the subscriber table says about {@code imsi} on {@code carrier}. */
        record Decision(byte[] key, CellFrame.Cause cause) {
            public static Decision reject(CellFrame.Cause c) {
                return new Decision(null, c);
            }
        }

        Decision authorize(String carrier, long imsi);

        /** {@code tower} now serves {@code imsi}: release it anywhere else. */
        void attached(TowerLink tower, long imsi);

        void detached(TowerLink tower, long imsi);

        /** Count {@code bytes} of data; false once the SIM's quota is used up. */
        boolean account(String carrier, long imsi, long bytes);
    }

    /** Where uplink frames go: the tower's cable segment. */
    public interface Backhaul {
        void toCable(byte[] ethernet);
    }

    /** A device as the tower sees it (the status screen's rows). */
    public record DeviceStatus(long imsi, String mac, int rnti, int slice, double ulRssiDbm, double ulSinrDb, double dlSinrDb,
                               int dlCqi, long bytesUp, long bytesDown, long attachedMillis, boolean handedIn) {}

    public static final int MAX_DEVICES = 64;
    static final long INACTIVITY_US = 6_000_000;
    static final long MAX_BACKLOG_US = 250_000;
    static final int QUEUE_LIMIT = 2048;
    /** Beacon offsets within a tick: 16 slots of 2.5 ms. */
    static final int BEACON_PHASES = 16;
    static final long BEACON_PHASE_US = 2_500;

    private final UUID id;
    private final int cellKey;
    private final PhyProfile tech;
    private final Supplier<RadioMedium> medium;
    private final Core core;
    private final Backhaul backhaul;
    private final Port[] ports;

    private volatile String carrier;
    private volatile int channel = 1;
    private volatile double txPowerDbm;
    private volatile Pose pose;
    private volatile AntennaPattern antenna = CellAntennas.tower(CellAntennas.panelFor(PhyProfile.LTE), List.of(), 0);
    private volatile boolean enabled = true;
    private RadioMedium registeredWith;

    private final ConcurrentLinkedQueue<Rx> airIn = new ConcurrentLinkedQueue<>();
    private final ConcurrentLinkedQueue<byte[]> cableIn = new ConcurrentLinkedQueue<>();
    private final AtomicInteger airQueued = new AtomicInteger(), cableQueued = new AtomicInteger();
    private final ConcurrentLinkedQueue<Long> dropRequests = new ConcurrentLinkedQueue<>();
    private final Map<Integer, Device> byRnti = new ConcurrentHashMap<>();
    private final Map<Long, Device> byMac = new ConcurrentHashMap<>();
    private int nextRnti = 1;
    private long nextFreeUs, lastBeaconUs = Long.MIN_VALUE;

    private final AtomicLong beacons = new AtomicLong(), dlFrames = new AtomicLong(), ulFrames = new AtomicLong(),
            dlDropped = new AtomicLong(), attaches = new AtomicLong(), rejects = new AtomicLong(), cableOut = new AtomicLong();
    private volatile String lastEvent = "";

    private record Rx(int slice, Reception reception) {}

    private static final class Device {
        final long imsi;
        final int rnti;
        final int slice;
        final byte[] mac;
        final long attachedUs;
        final boolean handedIn;
        volatile long lastHeardUs;
        volatile double dlSinrDb = Double.NaN, ulSinrDb = Double.NaN, ulRssiDbm = Double.NaN;
        final AtomicLong up = new AtomicLong(), down = new AtomicLong();

        Device(long imsi, int rnti, int slice, byte[] mac, long now, boolean handedIn) {
            this.imsi = imsi;
            this.rnti = rnti;
            this.slice = slice;
            this.mac = mac.clone();
            this.attachedUs = now;
            this.lastHeardUs = now;
            this.handedIn = handedIn;
        }
    }

    public TowerLink(UUID id, PhyProfile tech, Supplier<RadioMedium> medium, Core core, Backhaul backhaul) {
        this.id = id;
        this.cellKey = CellFrame.cellKey(id);
        this.tech = tech;
        this.medium = medium;
        this.core = core;
        this.backhaul = backhaul;
        this.txPowerDbm = tech.towerTxDbm;
        this.ports = new Port[tech.ulSlices];
        for (int i = 0; i < ports.length; i++) ports[i] = new Port(i, i == 0 ? id : derived(id, i));
    }

    private static UUID derived(UUID id, int i) {
        return new UUID(id.getMostSignificantBits() ^ (0x5A5AL * i), id.getLeastSignificantBits() + i);
    }

    // ------------------------------------------------------------ configuration (owner thread)

    /** Carrier (null = not serving) and downlink channel number. Devices are released when either changes. */
    public void configure(String carrier, int channel) {
        if (!tech.validChannel(channel)) throw new IllegalArgumentException(tech.displayName + " channel " + channel);
        boolean changed = !java.util.Objects.equals(carrier, this.carrier) || channel != this.channel;
        if (changed) releaseAll(CellFrame.Cause.SHUTDOWN);
        this.carrier = carrier;
        this.channel = channel;
        if (changed) invalidate();
    }

    /** Where the antennas are and what they look like; a null pose takes the tower off the air. */
    public void setAntenna(Pose pose, AntennaPattern pattern) {
        boolean newPattern = pattern != antenna;
        this.pose = pose;
        if (pattern != null) this.antenna = pattern;
        if (newPattern) invalidate();
    }

    public void setTxPowerDbm(double dbm) {
        txPowerDbm = Math.max(0, Math.min(tech.towerTxDbm, dbm));
    }

    public void setEnabled(boolean on) {
        if (enabled && !on) releaseAll(CellFrame.Cause.SHUTDOWN);
        enabled = on;
        invalidate();
    }

    private void invalidate() {
        RadioMedium m = registeredWith;
        if (m != null) for (Port p : ports) m.invalidate(p);
    }

    public UUID id() { return id; }
    public int cellKey() { return cellKey; }
    public PhyProfile tech() { return tech; }
    public String carrier() { return carrier; }
    public int channel() { return channel; }
    public double txPowerDbm() { return txPowerDbm; }
    public Pose pose() { return pose; }
    public AntennaPattern antenna() { return antenna; }
    public String lastEvent() { return lastEvent; }
    public long beacons() { return beacons.get(); }
    public long dlFrames() { return dlFrames.get(); }
    public long ulFrames() { return ulFrames.get(); }
    public long attaches() { return attaches.get(); }
    public long rejects() { return rejects.get(); }
    public long cableOut() { return cableOut.get(); }

    /** On the air: configured, placed and with at least one antenna. */
    public boolean onAir() {
        return enabled && carrier != null && pose != null && antenna.peakGainDbi() > -50;
    }

    /** The endpoint of uplink slice {@code i} (0 = the downlink transmitter too). */
    public RadioEndpoint endpoint(int i) {
        return ports[i];
    }

    public int deviceCount() {
        return byRnti.size();
    }

    public boolean serves(long imsi) {
        for (Device d : byRnti.values()) if (d.imsi == imsi) return true;
        return false;
    }

    public List<DeviceStatus> devices() {
        List<DeviceStatus> out = new ArrayList<>();
        long now = now();
        for (Device d : byRnti.values()) {
            out.add(new DeviceStatus(d.imsi, macText(d.mac), d.rnti, d.slice, d.ulRssiDbm, d.ulSinrDb, d.dlSinrDb,
                    CqiTable.cqiFor(tech, d.dlSinrDb), d.up.get(), d.down.get(), Math.max(0, (now - d.attachedUs) / 1000), d.handedIn));
        }
        out.sort(Comparator.comparingLong(DeviceStatus::imsi));
        return out;
    }

    // ------------------------------------------------------------ inputs (any thread)

    /** A frame seen on the tower's cable segment (the bridge port's sink). */
    public void fromCable(byte[] ethernet) {
        if (ethernet == null || ethernet.length < 14 || byRnti.isEmpty()) return;
        if (cableQueued.incrementAndGet() > QUEUE_LIMIT) {
            cableQueued.decrementAndGet();
            dlDropped.incrementAndGet();
            return;
        }
        cableIn.add(ethernet.clone());
    }

    /** The core moved {@code imsi} to another tower: forget it without telling the device. */
    public void dropDevice(long imsi) {
        dropRequests.add(imsi);
    }

    // ------------------------------------------------------------ server tick

    public void tick() {
        RadioMedium m = medium.get();
        if (m != registeredWith) {
            if (registeredWith != null) for (Port p : ports) registeredWith.unregister(p);
            registeredWith = m;
            if (m != null && pose != null) for (Port p : ports) m.register(p);
        } else if (m != null && pose != null) {
            for (Port p : ports) m.register(p);   // idempotent; covers a pose arriving after the medium
        }
        Long drop;
        while ((drop = dropRequests.poll()) != null) {
            for (Device d : List.copyOf(byRnti.values()))
                if (d.imsi == drop) {
                    forget(d);
                    lastEvent = SimCard.formatImsi(d.imsi) + " handed over to another tower";
                }
        }
        if (m == null || !onAir()) {
            airIn.clear();
            airQueued.set(0);
            cableIn.clear();
            cableQueued.set(0);
            return;
        }
        long now = m.nowMicros();
        if (lastBeaconUs == Long.MIN_VALUE || now - lastBeaconUs >= tech.beaconIntervalUs) {
            lastBeaconUs = now;
            beacons.incrementAndGet();
            // Towers tick together: each beacons at its own offset into the tick so neighbours' beacons don't always collide.
            send(m, CellFrame.beacon(cellKey, tech, channel, txPowerDbm, byRnti.size(), id, carrier), CqiTable.ROBUST_CQI,
                    Math.floorMod(cellKey, BEACON_PHASES) * BEACON_PHASE_US);
        }
        Rx rx;
        while ((rx = airIn.poll()) != null) {
            airQueued.decrementAndGet();
            onAir(m, rx, now);
        }
        byte[] eth;
        while ((eth = cableIn.poll()) != null) {
            cableQueued.decrementAndGet();
            down(m, eth);
        }
        for (Device d : List.copyOf(byRnti.values())) {
            if (now - d.lastHeardUs > INACTIVITY_US) {
                forget(d);
                core.detached(this, d.imsi);
                lastEvent = SimCard.formatImsi(d.imsi) + " went quiet (no signal for 6 s)";
            }
        }
    }

    /** Off the air for good: release everyone and leave the medium. */
    public void stop() {
        releaseAll(CellFrame.Cause.SHUTDOWN);
        if (registeredWith != null) for (Port p : ports) registeredWith.unregister(p);
        registeredWith = null;
    }

    // ------------------------------------------------------------ uplink

    private void onAir(RadioMedium m, Rx rx, long now) {
        CellFrame.Frame f = CellFrame.decode(rx.reception.payload());
        if (f == null || f.cellKey() != cellKey) return;
        switch (f) {
            case CellFrame.Attach a -> attach(m, a, now);
            case CellFrame.Data d when d.uplink() -> {
                Device dev = byRnti.get(d.rnti());
                if (dev == null) {
                    send(m, CellFrame.release(cellKey, d.rnti(), CellFrame.Cause.NONE), CqiTable.ROBUST_CQI);
                    return;
                }
                heard(dev, rx.reception, d.dlSinrDb(), now);
                ulFrames.incrementAndGet();
                if (!account(m, dev, d.ethernet().length, true)) return;
                up(m, dev, d.ethernet());
            }
            case CellFrame.Report r -> {
                Device dev = byRnti.get(r.rnti());
                if (dev == null) send(m, CellFrame.release(cellKey, r.rnti(), CellFrame.Cause.NONE), CqiTable.ROBUST_CQI);
                else heard(dev, rx.reception, r.dlSinrDb(), now);
            }
            case CellFrame.Detach dt -> {
                Device dev = byRnti.get(dt.rnti());
                if (dev != null) {
                    forget(dev);
                    core.detached(this, dev.imsi);
                    lastEvent = SimCard.formatImsi(dev.imsi) + " detached";
                }
            }
            default -> { }
        }
    }

    private void heard(Device d, Reception r, double dlSinr, long now) {
        d.lastHeardUs = now;
        d.ulRssiDbm = r.rssiDbm();
        d.ulSinrDb = r.sinrDb();
        if (!Double.isNaN(dlSinr)) d.dlSinrDb = dlSinr;
    }

    private void attach(RadioMedium m, CellFrame.Attach a, long now) {
        String c = carrier;
        Core.Decision dec = core.authorize(c, a.imsi());
        CellFrame.Cause cause = dec == null ? CellFrame.Cause.UNKNOWN_SIM : dec.cause();
        if (cause == CellFrame.Cause.NONE && !SimCard.proofEquals(a.proof(),
                SimCard.attachProof(dec.key(), cellKey, a.imsi(), a.mac(), a.nonce())))
            cause = CellFrame.Cause.BAD_KEY;
        Device existing = null;
        for (Device d : byRnti.values()) if (d.imsi == a.imsi()) existing = d;
        if (cause == CellFrame.Cause.NONE && existing == null && byRnti.size() >= MAX_DEVICES) cause = CellFrame.Cause.FULL;
        if (cause != CellFrame.Cause.NONE) {
            rejects.incrementAndGet();
            lastEvent = "refused " + SimCard.formatImsi(a.imsi()) + ": " + cause.text;
            send(m, CellFrame.reject(cellKey, a.imsi(), cause), CqiTable.ROBUST_CQI);
            return;
        }
        Device dev;
        if (existing != null && Arrays.equals(existing.mac, a.mac())) {
            dev = existing;   // a retransmitted request (our accept got lost): same radio id
        } else {
            if (existing != null) forget(existing);
            Device clash = byMac.get(macKey(a.mac()));
            if (clash != null) forget(clash);
            dev = new Device(a.imsi(), allocateRnti(), quietestSlice(), a.mac(), now, a.handover());
            byRnti.put(dev.rnti, dev);
            byMac.put(macKey(dev.mac), dev);
            attaches.incrementAndGet();
            lastEvent = SimCard.formatImsi(a.imsi()) + (a.handover() ? " handed in" : " attached") + " (slice " + dev.slice + ")";
        }
        dev.lastHeardUs = now;
        if (!Double.isNaN(a.dlSinrDb())) dev.dlSinrDb = a.dlSinrDb();
        core.attached(this, a.imsi());
        send(m, CellFrame.accept(cellKey, a.imsi(), dev.rnti, dev.slice,
                SimCard.acceptProof(dec.key(), cellKey, a.imsi(), a.nonce(), dev.rnti)), CqiTable.ROBUST_CQI);
    }

    private int allocateRnti() {
        for (int i = 0; i < 0xFFFE; i++) {
            int r = nextRnti;
            nextRnti = nextRnti >= 0xFFFE ? 1 : nextRnti + 1;
            if (!byRnti.containsKey(r)) return r;
        }
        throw new IllegalStateException("no free RNTI");
    }

    private int quietestSlice() {
        int[] load = new int[ports.length];
        for (Device d : byRnti.values()) load[d.slice]++;
        int best = 0;
        for (int i = 1; i < load.length; i++) if (load[i] < load[best]) best = i;
        return best;
    }

    /** An uplink Ethernet frame: to another device on this tower, or onto the cable. */
    private void up(RadioMedium m, Device from, byte[] eth) {
        if (eth.length < 14) return;
        boolean group = (eth[0] & 1) != 0;
        if (!group) {
            Device to = byMac.get(macKey(eth));
            if (to != null && to != from) {
                if (account(m, to, eth.length, false)) send(m, CellFrame.dataDownlink(cellKey, to.rnti, eth), CqiTable.cqiFor(tech, to.dlSinrDb));
                return;
            }
        } else if (byRnti.size() > 1) {
            send(m, CellFrame.dataDownlink(cellKey, CellFrame.BROADCAST_RNTI, eth), broadcastCqi());
        }
        cableOut.incrementAndGet();
        backhaul.toCable(eth);
    }

    // ------------------------------------------------------------ downlink

    /** A cable frame for an attached device (or everyone). */
    private void down(RadioMedium m, byte[] eth) {
        if ((eth[0] & 1) != 0) {
            if (!byRnti.isEmpty()) send(m, CellFrame.dataDownlink(cellKey, CellFrame.BROADCAST_RNTI, eth), broadcastCqi());
            return;
        }
        Device d = byMac.get(macKey(eth));
        if (d == null) return;
        if (!account(m, d, eth.length, false)) return;
        send(m, CellFrame.dataDownlink(cellKey, d.rnti, eth), CqiTable.cqiFor(tech, d.dlSinrDb));
    }

    private int broadcastCqi() {
        double worst = Double.POSITIVE_INFINITY;
        for (Device d : byRnti.values()) worst = Math.min(worst, Double.isNaN(d.dlSinrDb) ? Double.NEGATIVE_INFINITY : d.dlSinrDb);
        return Double.isInfinite(worst) && worst < 0 ? CqiTable.ROBUST_CQI : CqiTable.cqiFor(tech, worst);
    }

    /** Count bytes on the SIM; release the device once its quota is used up. */
    private boolean account(RadioMedium m, Device d, int bytes, boolean uplink) {
        (uplink ? d.up : d.down).addAndGet(bytes);
        if (core.account(carrier, d.imsi, bytes)) return true;
        lastEvent = SimCard.formatImsi(d.imsi) + " reached its data limit";
        send(m, CellFrame.release(cellKey, d.rnti, CellFrame.Cause.QUOTA), CqiTable.ROBUST_CQI);
        forget(d);
        core.detached(this, d.imsi);
        return false;
    }

    /** Put a downlink frame on the air at {@code cqi}, after whatever is already queued. */
    private void send(RadioMedium m, byte[] payload, int cqi) {
        send(m, payload, cqi, 0);
    }

    private void send(RadioMedium m, byte[] payload, int cqi, long delayUs) {
        Pose p = pose;
        if (m == null || p == null) return;
        long now = m.nowMicros();
        long start = Math.max(now + delayUs, nextFreeUs);
        if (start - now > MAX_BACKLOG_US) {
            dlDropped.incrementAndGet();
            return;
        }
        double rate = CqiTable.downlinkRateBps(tech, cqi);
        long dur = CqiTable.airtimeUs(tech, payload.length, rate);
        nextFreeUs = start + dur;
        dlFrames.incrementAndGet();
        m.transmit(ports[0], Emission.frame(tech.downlink(channel), txPowerDbm, start, dur, CqiTable.modulation(cqi), rate, payload));
    }

    private void releaseAll(CellFrame.Cause cause) {
        RadioMedium m = registeredWith;
        for (Device d : List.copyOf(byRnti.values())) {
            if (m != null && onAir()) send(m, CellFrame.release(cellKey, d.rnti, cause), CqiTable.ROBUST_CQI);
            forget(d);
            core.detached(this, d.imsi);
        }
    }

    private void forget(Device d) {
        byRnti.remove(d.rnti, d);
        byMac.remove(macKey(d.mac), d);
    }

    // ------------------------------------------------------------ endpoints

    private long now() {
        RadioMedium m = medium.get();
        return m == null ? 0 : m.nowMicros();
    }

    static long macKey(byte[] b) {
        long v = 0;
        for (int i = 0; i < 6; i++) v = v << 8 | (b[i] & 0xff);
        return v;
    }

    static String macText(byte[] m) {
        return String.format("%02x:%02x:%02x:%02x:%02x:%02x", m[0] & 0xff, m[1] & 0xff, m[2] & 0xff, m[3] & 0xff, m[4] & 0xff, m[5] & 0xff);
    }

    /** One uplink slice's receiver (slice 0 also transmits the downlink). */
    private final class Port implements RadioEndpoint {
        final int slice;
        final UUID pid;

        Port(int slice, UUID pid) {
            this.slice = slice;
            this.pid = pid;
        }

        @Override public UUID id() { return pid; }
        @Override public Pose pose() { return pose; }
        @Override public AntennaPattern antenna() { return antenna; }
        @Override public Channel tunedChannel() { return onAir() ? tech.uplinkSlice(channel, slice) : null; }
        @Override public double noiseFigureDb() { return tech.towerNoiseFigureDb; }
        @Override public double sensitivityDbm() { return CqiTable.towerSensitivityDbm(tech); }
        @Override public double maxTxPowerDbm() { return tech.towerTxDbm; }

        @Override
        public void onReceive(Reception r) {
            Emission e = r.emission();
            if (e.kind() != Emission.Kind.FRAME || e.payload() == null || CqiTable.cqiOf(e.modulation()) == 0) return;
            if (airQueued.incrementAndGet() > QUEUE_LIMIT) {
                airQueued.decrementAndGet();
                return;
            }
            airIn.add(new Rx(slice, r));
        }
    }
}
