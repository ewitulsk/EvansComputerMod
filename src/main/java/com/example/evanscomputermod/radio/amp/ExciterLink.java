package com.example.evanscomputermod.radio.amp;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.RadioConfig;
import com.example.evanscomputermod.radio.antenna.Antenna;
import com.example.evanscomputermod.radio.antenna.AntennaManager;
import com.example.evanscomputermod.radio.antenna.graph.AntennaReport;
import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.hazard.ChainHazards;
import com.example.evanscomputermod.radio.hazard.RadioOwners;
import com.example.evanscomputermod.radio.hazard.RfExposure;
import net.minecraft.core.BlockPos;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.Level;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicLong;

/**
 * An exciter's view of its {@link TransmitChain} (one per SDR): the
 * "transmit chain resolver". When anything is attached to the exciter (an
 * amplifier touching it, coax, a tuner, a feed point) the exciter radiates
 * from the antenna at the end of the chain instead of its built-in whip:
 * <ul>
 *   <li>{@link #pose()}: the antenna's feed point pose (Sable-aware);</li>
 *   <li>{@link #pattern}: the antenna's solved pattern at the frequency, whose
 *       {@code feedLossDb} is the antenna's mismatch plus every feedline run's
 *       loss (used for receive too: the SDR hears through the same antenna);</li>
 *   <li>{@link #transmit}: rewrites each emission's power to the amplifier's
 *       output (brownout, foldback) and keys the amplifier up.</li>
 * </ul>
 * Each server tick it re-reads the antenna, spreads the airtime of what was
 * sent over ticks, deposits heat and FE draw in the amplifier and tuner, and
 * runs the chain's thermal hazards. Thread-safe where it has to be:
 * {@link #transmit}, {@link #pattern} and {@link #pose} are called from
 * computer and medium threads and only read the latest immutable snapshot.
 */
public final class ExciterLink {
    private static final long TICK_MICROS = 50_000;
    private static final int RESOLVE_EVERY = 4;

    private record Key(ResourceKey<Level> dim, BlockPos pos) {}

    private record Snapshot(TransmitChain chain, @Nullable Antenna antenna, @Nullable Pose pose,
                            @Nullable AmplifierBlockEntity amp, @Nullable TunerBlockEntity tuner) {}

    private record PatternCache(Snapshot snap, double hz, AntennaPattern pattern) {}

    /** The antenna pattern plus the chain's feedline loss. */
    private record Lossy(AntennaPattern base, double extraLossDb) implements AntennaPattern {
        @Override public double gainDbi(double lx, double ly, double lz) { return base.gainDbi(lx, ly, lz); }
        @Override public double[] polarization(double lx, double ly, double lz) { return base.polarization(lx, ly, lz); }
        @Override public double peakGainDbi() { return base.peakGainDbi(); }
        @Override public double feedLossDb() { return base.feedLossDb() + extraLossDb; }
    }

    private static final Snapshot EMPTY = new Snapshot(TransmitChain.NONE, null, null, null, null);
    private static final Map<Key, ExciterLink> LINKS = new ConcurrentHashMap<>();

    private final UUID id;
    private volatile Snapshot snap = EMPTY;
    private volatile PatternCache patternCache;
    private final AtomicLong pendingAir = new AtomicLong();
    private volatile ChainBudget lastBudget;
    private volatile double lastDriveW, lastHz;
    private final ChainHazards hazards = new ChainHazards();
    private volatile String lastHazard = "";
    @Nullable private volatile UUID owner;
    @Nullable private Key key;
    private int resolveIn;

    /** @param id the exciter's endpoint id (the RadioTransmitEvent source) */
    public ExciterLink(UUID id) {
        this.id = id;
    }

    // ------------------------------------------------------------ registry

    /** Every link with something attached, in a level. */
    public static List<ExciterLink> in(Level level) {
        List<ExciterLink> out = new ArrayList<>();
        for (var e : LINKS.entrySet()) if (e.getKey().dim == level.dimension()) out.add(e.getValue());
        return out;
    }

    /** The link whose chain runs through the device (amplifier, tuner, feed point) at {@code device}, or null. */
    @Nullable
    public static ExciterLink through(Level level, BlockPos device) {
        for (ExciterLink l : in(level)) {
            for (TransmitChain.Hop h : l.chain().hops()) if (device.equals(h.pos())) return l;
            if (l.chain().lineBlocks().contains(device)) return l;
        }
        return null;
    }

    // ------------------------------------------------------------ reads (any thread)

    public boolean connected() { return snap.chain.connected(); }
    public TransmitChain chain() { return snap.chain; }
    @Nullable public Antenna antenna() { return snap.antenna; }
    @Nullable public AmplifierBlockEntity amplifier() { return snap.amp; }
    @Nullable public TunerBlockEntity tuner() { return snap.tuner; }
    @Nullable public UUID owner() { return owner; }
    @Nullable public ChainBudget lastBudget() { return lastBudget; }
    public ChainHazards hazards() { return hazards; }

    /** World pose of the antenna (feed point) when connected, else null (use the whip). */
    @Nullable
    public Pose pose() {
        Snapshot s = snap;
        return s.chain.connected() ? s.pose : null;
    }

    /** The pattern to use at {@code hz} when connected (antenna + all feedline loss), else null (use the whip). */
    @Nullable
    public AntennaPattern pattern(double hz) {
        Snapshot s = snap;
        if (!s.chain.connected()) return null;
        PatternCache c = patternCache;
        if (c != null && c.snap == s && c.hz == hz) return c.pattern;
        AntennaPattern base = s.antenna != null && s.chain.feed() != null ? s.antenna.pattern(hz)
                : AntennaReport.none("open feedline").patternAt(hz);
        AntennaPattern p = new Lossy(base, s.chain.totalLineLossDb(hz));
        patternCache = new PatternCache(s, hz, p);
        return p;
    }

    /** Highest power the endpoint can put out, dBm (for the medium's culling). */
    public double maxTxDbm(double exciterMaxDbm) {
        AmplifierBlockEntity amp = snap.amp;
        return amp == null ? exciterMaxDbm : Math.max(exciterMaxDbm, AmpModel.wToDbm(amp.tier().ratedW));
    }

    /**
     * The exciter is about to transmit {@code e}: returns the emission as it
     * leaves the chain (power = amplifier output, see {@link ChainBudget}), or
     * null if an "amplifier" RadioTransmitEvent cancelled the burst. Called on
     * the transmitting thread.
     */
    @Nullable
    public Emission transmit(Emission e) {
        Snapshot s = snap;
        double drive = AmpModel.dbmToW(e.powerDbm()), hz = e.channel().centerHz();
        lastDriveW = drive;
        lastHz = hz;
        if (!s.chain.connected()) {
            lastBudget = null;
            pendingAir.addAndGet(e.durationMicros());
            return e;
        }
        AmplifierBlockEntity amp = s.amp;
        ChainBudget b = ChainBudget.compute(s.chain, s.antenna, hz, drive, amp == null ? null : amp.tier(),
                amp == null ? 0 : amp.supply(), amp == null ? Double.POSITIVE_INFINITY : amp.limitW());
        if (amp != null && b.amp() != null && !amp.keyUp(e, b, id, s.pose)) return null;
        lastBudget = b;
        pendingAir.addAndGet(e.durationMicros());
        return new Emission(e.kind(), e.channel(), b.emissionDbm(), e.startMicros(), e.durationMicros(), e.modulation(),
                e.bitRate(), e.payload(), e.iq(), e.sampleRateHz());
    }

    // ------------------------------------------------------------ server tick

    /**
     * One server tick for the exciter at {@code pos} ({@code whipPose}: its
     * own antenna, for the RF meter when nothing is attached). Returns true
     * when the endpoint's antenna or pose changed (the caller invalidates its
     * medium links).
     */
    public boolean tick(ServerLevel level, BlockPos pos, @Nullable Pose whipPose) {
        key = new Key(level.dimension(), pos.immutable());
        Snapshot old = snap;
        TransmitChain chain = old.chain;
        boolean stale = (old.amp != null && old.amp.isRemoved()) || (old.tuner != null && old.tuner.isRemoved());
        if (--resolveIn <= 0 || stale || !chain.exciter().equals(pos)) {
            chain = TransmitChain.resolve(level, pos);
            resolveIn = RESOLVE_EVERY;
        }
        BlockPos feed = chain.feed();
        Antenna antenna = feed != null ? AntennaManager.get(level, feed) : null;
        Pose pose = antenna != null ? antenna.pose(level) : whipPose;
        AmplifierBlockEntity amp = null;
        TunerBlockEntity tuner = null;
        int ai = chain.ampIndex(), ti = chain.tunerIndex();
        if (ai >= 0 && level.getBlockEntity(chain.hops().get(ai).pos()) instanceof AmplifierBlockEntity a) amp = a;
        if (ti >= 0 && level.getBlockEntity(chain.hops().get(ti).pos()) instanceof TunerBlockEntity t) tuner = t;
        Snapshot now = new Snapshot(chain, antenna, pose, amp, tuner);
        snap = now;
        // Movement is not a change here: the medium watches the endpoint's pose under its rate limiter.
        boolean changed = !chain.equals(old.chain) || version(antenna) != version(old.antenna);
        if (chain.connected()) LINKS.put(key, this);
        else LINKS.remove(key);
        UUID o = RadioOwners.get(level, feed);
        if (o == null && amp != null) o = amp.owner();
        owner = o;

        long air = pendingAir.getAndUpdate(v -> v - Math.min(v, TICK_MICROS));
        double duty = Math.min(air, TICK_MICROS) / (double) TICK_MICROS;
        ChainBudget b = lastBudget;
        long tick = level.getGameTime();
        String dim = level.dimension().location().toString();
        if (duty > 0) {
            if (chain.connected() && b != null) {
                if (amp != null && b.amp() != null)
                    amp.deposit(b.amp().heatW() * duty, AmpModel.feDrawPerTick(b.amp().amplifiedW(), duty, RadioConfig.wattsPerFePerTick()), b, feed);
                if (tuner != null)
                    tuner.deposit(b.tunerHeatW() * duty, AmpModel.swrOf(b.rhoAntenna()), b.tunerMatched(), feed);
                if (pose != null && b.acceptedW() > 0) {
                    AntennaPattern p = antenna != null ? antenna.pattern(b.hz()) : AntennaPattern.ISOTROPIC;
                    RfExposure.update(this, new RfExposure.Source(dim, pose, p, b.acceptedW(), b.hz(), tick + 2));
                }
            } else if (whipPose != null) {
                RfExposure.update(this, new RfExposure.Source(dim, whipPose, AntennaPattern.VERTICAL_DIPOLE, lastDriveW, lastHz, tick + 2));
            }
        }
        if (chain.connected()) hazards.tick(level, chain, antenna, b, duty, amp, owner);
        return changed;
    }

    private static long version(@Nullable Antenna a) {
        return a == null ? -1 : a.version();
    }

    /** The exciter is gone. */
    public void remove() {
        if (key != null) LINKS.remove(key);
        RfExposure.remove(this);
    }

    /** Records a hazard outcome for the status lines (lightning, etc.). */
    public void noteHazard(String what) {
        lastHazard = what;
    }

    /** Chain warnings: hot parts, missing arrestor, last hazard. */
    public List<String> warnings() {
        List<String> out = new ArrayList<>(hazards.status());
        Snapshot s = snap;
        if (s.chain.feed() == null && s.chain.connected()) out.add("Feedline ends open: no antenna");
        else if (s.chain.connected() && !s.chain.hasArrestor()) out.add("No lightning arrestor in the feedline");
        Antenna a = s.antenna;
        ChainBudget b = lastBudget;
        if (a != null && a.present() && b != null) {
            double t = com.example.evanscomputermod.radio.hazard.ThermalModel.Part.WIRE.timeToFailure(hazards.wireTheta(),
                    b.acceptedW() / Math.max(1e-9, a.wireLimitW()));
            out.add(String.format(Locale.ROOT, "Antenna takes %s of %s rated (%s)%s", AmplifierBlockEntity.w(b.acceptedW()),
                    AmplifierBlockEntity.w(a.powerLimitW()), a.weakestLink(),
                    Double.isFinite(t) ? " → wire fails after " + com.example.evanscomputermod.radio.hazard.ThermalModel.describeSeconds(t) + " continuous" : ""));
        }
        if (!hazards.lastEvent().isEmpty()) out.add("Last hazard: " + hazards.lastEvent());
        if (!lastHazard.isEmpty()) out.add(lastHazard);
        return out;
    }
}
//?}
