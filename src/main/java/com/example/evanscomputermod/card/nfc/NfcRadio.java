package com.example.evanscomputermod.card.nfc;

import com.example.evanscomputermod.card.Hex;
import com.example.evanscomputermod.card.applet.NfcApplet;
import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.function.Supplier;

/**
 * An NFC reader's silicon, shared by the NFC Reader block and the NFC module: a
 * {@link RadioEndpoint} on the near-field band that energises cards (an ENERGY emission
 * every tick while the field is on) and exchanges frames with them. Commands are
 * synchronous: the medium delivers a frame to the energised cards in the call, each
 * card's reply comes straight back into the inbox, and the reader collects what arrived
 * in the reply window. The reader side of the sector handshake lives here too, since it
 * is what any reader does with a key; the policy (which key, which sector, what the data
 * means) is the computer's program.
 */
public final class NfcRadio {
    public static final double MIN_DBM = 0, DEFAULT_DBM = 0;
    public static final double SENSITIVITY_DBM = -90;
    public static final long FIELD_TICK_MICROS = 50_000;

    /** Who wants to know when a field goes on or off (the {@code CardEndpointTracker}); no-ops in tests. */
    public static volatile java.util.function.Consumer<NfcRadio> onFieldOn = r -> {}, onFieldOff = r -> {};

    private final UUID id;
    private final Supplier<Pose> pose;
    private final Supplier<RadioMedium> media;
    private final double maxDbm;
    private final Endpoint endpoint = new Endpoint();
    private final ConcurrentLinkedQueue<Reception> inbox = new ConcurrentLinkedQueue<>();
    private volatile boolean field;
    private volatile double txDbm = DEFAULT_DBM;
    private RadioMedium registeredWith;
    private volatile long selectedUid;
    private volatile double lastRssi = Double.NaN;
    private final Map<Long, Long> seen = new LinkedHashMap<>();
    private volatile boolean live;

    /** {@code media} gives the current medium (the server's {@code RadioMediumHooks.medium()}, or a test's). */
    public NfcRadio(UUID id, Supplier<Pose> pose, Supplier<RadioMedium> media, double maxDbm) {
        this.id = id;
        this.pose = pose;
        this.media = media;
        this.maxDbm = maxDbm;
        this.txDbm = maxDbm;   // full field until a program turns it down
    }

    public UUID id() {
        return id;
    }

    public RadioEndpoint endpoint() {
        return endpoint;
    }

    // ------------------------------------------------------------ life cycle (server thread)

    public void load() {
        live = true;
        RadioMedium m = media.get();
        if (m != null) {
            m.register(endpoint);
            registeredWith = m;
        }
        if (field) onFieldOn.accept(this);
    }

    public void unload() {
        live = false;
        onFieldOff.accept(this);
        if (registeredWith != null) registeredWith.unregister(endpoint);
        registeredWith = null;
    }

    /** Once a tick: keep the field up while it is on. */
    private Pose lastPose;

    public void tick() {
        if (!field || !live) return;
        RadioMedium m = medium();
        if (m == null) return;
        Pose p = pose.get();
        if (p != null && (lastPose == null || !p.equals(lastPose))) {
            lastPose = p;
            m.invalidate(endpoint);   // the coil moved (a phone's module): the medium re-traces its links
        }
        long now = m.nowMicros();
        m.transmit(endpoint, Emission.energy(NfcAir.CHANNEL, txDbm, now, FIELD_TICK_MICROS));
    }

    private @Nullable RadioMedium medium() {
        RadioMedium m = media.get();
        if (m != null && m != registeredWith) {
            m.register(endpoint);
            registeredWith = m;
        }
        return m;
    }

    // ------------------------------------------------------------ settings

    public boolean field() {
        return field;
    }

    public void setField(boolean on) {
        if (field == on) return;
        field = on;
        if (on) {
            onFieldOn.accept(this);
            tick();
        } else {
            onFieldOff.accept(this);
            selectedUid = 0;
            seen.clear();
        }
    }

    public double txDbm() {
        return txDbm;
    }

    public void setTxDbm(double dbm) {
        txDbm = Math.max(MIN_DBM, Math.min(maxDbm, dbm));
    }

    public double maxDbm() {
        return maxDbm;
    }

    public Pose pose() {
        return pose.get();
    }

    public double lastRssi() {
        return lastRssi;
    }

    /** Receptions the antenna saw (frames, before the inbox) — diagnostics. */
    public volatile long receptions;

    /** The medium's view of the link to {@code other}: path gain and the cached link (diagnostics). */
    public String debugLink(RadioEndpoint other) {
        RadioMedium m = media.get();
        if (m == null) return "no medium";
        String link = m instanceof com.example.evanscomputermod.radio.medium.WorldRadioMedium w
                ? String.valueOf(w.link(endpoint, other, NfcAir.CHANNEL.centerHz())) : "-";
        String stats = m instanceof com.example.evanscomputermod.radio.medium.WorldRadioMedium w
                ? String.format(" (endpoints %d, cached %d, queued %d, computed last tick %d, medium tick %d; reader %s; card %s)", w.endpointCount(), w.cachedLinks(),
                        w.queuedLinks(), w.computedLastTick(), w.lastTick(), w.debugNode(endpoint), w.debugNode(other)) : "";
        return String.format("gain %.1f dB, link %s%s", m.pathGainDb(endpoint, other, NfcAir.CHANNEL.centerHz()), link, stats);
    }

    public long selectedUid() {
        return selectedUid;
    }

    /** UIDs heard in the field with the tick they were last heard (the tracker ages them). */
    public Map<Long, Long> seen() {
        return seen;
    }

    // ------------------------------------------------------------ commands (server thread)

    /** Send a command and collect every reply that arrives in the reply window. */
    public List<NfcAir.Frame> exchange(NfcAir.Frame cmd) {
        RadioMedium m = medium();
        List<NfcAir.Frame> replies = new ArrayList<>();
        if (m == null || !field) return replies;
        inbox.clear();
        byte[] bytes = cmd.encode();
        long now = m.nowMicros();
        // the field must be up for the cards to answer: top it up before every command
        m.transmit(endpoint, Emission.energy(NfcAir.CHANNEL, txDbm, now, FIELD_TICK_MICROS));
        m.transmit(endpoint, Emission.frame(NfcAir.CHANNEL, txDbm, now + 1, NfcAir.durationMicros(bytes.length),
                NfcAir.MODULATION, NfcAir.BIT_RATE, bytes));
        Reception r;
        while ((r = inbox.poll()) != null) {
            NfcAir.Frame f = NfcAir.decode(r.payload());
            if (f == null) continue;
            lastRssi = r.rssiDbm();
            replies.add(f);
        }
        return replies;
    }

    /** One reply, or null (silence, or an error frame when {@code errors} is false). */
    public @Nullable NfcAir.Frame one(NfcAir.Frame cmd) {
        for (NfcAir.Frame f : exchange(cmd)) return f;
        return null;
    }

    /** POLL a few rounds: every UID that answered, with its kinds. */
    public Map<Long, Integer> poll(int rounds) {
        Map<Long, Integer> found = new LinkedHashMap<>();
        for (int i = 0; i < rounds; i++) {
            for (NfcAir.Frame f : exchange(NfcAir.frame(NfcAir.POLL))) {
                if (f.cmd() != (NfcAir.POLL | NfcAir.REPLY) || f.body().length != 9) continue;
                found.put(Hex.toLong(f.slice(0, 8)), f.body()[8] & 0xff);
            }
        }
        return found;
    }

    public boolean select(long uid) {
        NfcAir.Frame r = one(NfcAir.frame(NfcAir.SELECT, Hex.longBytes(uid)));
        boolean ok = r != null && r.cmd() == (NfcAir.SELECT | NfcAir.REPLY);
        selectedUid = ok ? uid : 0;
        return ok;
    }

    /** The reader side of the 3-pass handshake for {@code sector} with {@code key}; null = opened, else the reason. */
    public @Nullable String auth(int sector, byte[] key, java.util.Random rnd) {
        if (selectedUid == 0) return "no card selected";
        byte[] r1 = new byte[8];
        rnd.nextBytes(r1);
        NfcAir.Frame a = one(NfcAir.frame(NfcAir.AUTH, new byte[] {(byte) sector}, r1));
        if (a == null) return "no answer";
        if (a.isError()) return reason(a);
        if (a.cmd() != (NfcAir.AUTH | NfcAir.REPLY) || a.body().length != 24) return "bad answer";
        byte[] r2 = a.slice(0, 8), m1 = a.slice(8, 16);
        if (!java.security.MessageDigest.isEqual(m1, NfcApplet.m1(key, r1, r2))) return "the card does not have this key";
        NfcAir.Frame b = one(NfcAir.frame(NfcAir.AUTH2, NfcApplet.m2(key, r1, r2)));
        if (b == null) return "no answer";
        if (b.isError()) return reason(b);
        return null;
    }

    public static String reason(NfcAir.Frame f) {
        return switch (f.errorReason()) {
            case NfcAir.ERR_BAD_AUTH -> "authentication failed";
            case NfcAir.ERR_NO_SESSION -> "sector not open";
            case NfcAir.ERR_BAD_SECTOR -> "bad sector";
            case NfcAir.ERR_NOT_SELECTED -> "no card selected";
            default -> "malformed";
        };
    }

    /** The reader's antenna on the medium. */
    private final class Endpoint implements RadioEndpoint {
        @Override
        public UUID id() {
            return id;
        }

        @Override
        public Pose pose() {
            return pose.get();
        }

        @Override
        public AntennaPattern antenna() {
            return AntennaPattern.ISOTROPIC;
        }

        @Override
        public Channel tunedChannel() {
            return live && field ? NfcAir.CHANNEL : null;
        }

        @Override
        public double maxTxPowerDbm() {
            return maxDbm;
        }

        @Override
        public double sensitivityDbm() {
            return SENSITIVITY_DBM;
        }

        @Override
        public void onReceive(Reception reception) {
            if (reception.emission().kind() != Emission.Kind.FRAME) return;
            receptions++;
            if (inbox.size() < 64) inbox.offer(reception);
        }
    }
}
