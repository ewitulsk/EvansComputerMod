package com.example.evanscomputermod.radio.amp;

//? if <=1.21.1 {
import com.example.evanscomputermod.energy.RadioEnergyStorage;
import com.example.evanscomputermod.radio.RadioConfig;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.event.HazardEvent;
import com.example.evanscomputermod.radio.api.event.RadioTransmitEvent;
import com.example.evanscomputermod.radio.hazard.HazardActions;
import com.example.evanscomputermod.radio.hazard.RadioHazardContent;
import com.example.evanscomputermod.radio.hazard.ThermalModel;
import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.particles.ParticleTypes;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.sounds.SoundEvents;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.neoforged.neoforge.common.NeoForge;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.UUID;

/**
 * Power amplifier state: FE buffer, supply fraction (brownout), temperature,
 * arc foldback and what the last transmission through it looked like.
 *
 * <p>Threads: the exciter's transmit hook (a computer's thread) reads
 * {@link #supply()} / {@link #limitW()} and calls {@link #keyUp}; everything
 * else runs on the server thread. Heat and FE draw are deposited by the
 * exciter's {@link ExciterLink} tick and integrated in this block entity's
 * own tick.
 */
public class AmplifierBlockEntity extends BlockEntity {
    /** A gap this long between emissions ends a transmission burst. */
    public static final long BURST_GAP_MICROS = 100_000;
    /** Arc foldback lasts this long. */
    public static final int ARC_FOLDBACK_TICKS = 200;
    /** Seconds of full-power operation the FE buffer holds. */
    public static final int BUFFER_SECONDS = 20;

    private final AmpModel.Tier tier;
    private final RadioEnergyStorage energy;
    private volatile double supply;
    private double theta;
    private double pendingHeatW, pendingFe, feCarry;
    private volatile double lastFePerTick;
    private volatile ChainBudget lastBudget;
    private long lastTxTick = Long.MIN_VALUE;
    private volatile double arcLimitW = Double.POSITIVE_INFINITY;
    private long arcUntil;
    private long lastTxEndMicros = Long.MIN_VALUE;
    private boolean burstCancelled;
    private long bursts;
    private long feDrawnTotal;
    private boolean warnedSwr;
    @Nullable private UUID owner;
    @Nullable private BlockPos lastFeed;

    public AmplifierBlockEntity(BlockPos pos, BlockState state) {
        super(RadioAmpContent.AMPLIFIER_BE.get(), pos, state);
        this.tier = state.getBlock() instanceof AmplifierBlock b ? b.tier() : AmpModel.Tier.W100;
        int perTick = fullFePerTick(tier);
        this.energy = new RadioEnergyStorage(perTick * 20 * BUFFER_SECONDS, perTick * 4, 0, this::setChanged);
    }

    /** FE per tick at full output while transmitting (40 / 400 / 4000 at 5 W per FE/t). */
    public static int fullFePerTick(AmpModel.Tier tier) {
        return (int) Math.ceil(AmpModel.feDrawPerTick(tier.ratedW, 1, RadioConfig.wattsPerFePerTick()));
    }

    private AmplifierPeripheral peripheral;

    public AmplifierPeripheral peripheral() {
        if (peripheral == null) peripheral = new AmplifierPeripheral(this);
        return peripheral;
    }

    public AmpModel.Tier tier() { return tier; }
    public RadioEnergyStorage energy() { return energy; }
    public double supply() { return supply; }
    public double theta() { return theta; }
    public double temperatureC() { return ThermalModel.Part.AMPLIFIER.celsius(theta); }
    @Nullable public ChainBudget lastBudget() { return lastBudget; }
    public double lastFePerTick() { return lastFePerTick; }
    public long bursts() { return bursts; }
    /** FE drawn for transmissions since the block entity loaded. */
    public long feDrawnTotal() { return feDrawnTotal; }
    @Nullable public UUID owner() { return owner; }

    public void setOwner(@Nullable UUID owner) {
        this.owner = owner;
        setChanged();
    }

    /** Extra output limit from a recent arc, W (+∞ when none). */
    public double limitW() { return arcLimitW; }

    /** True while transmitting (an emission ended within the last second). */
    public boolean transmitting() {
        return level != null && level.getGameTime() - lastTxTick <= 20;
    }

    /**
     * An emission is about to go through (called on the transmitting thread).
     * The first emission of a burst posts a {@link RadioTransmitEvent}
     * ("amplifier"); a cancelled burst stays blocked until the next gap.
     * Returns false if blocked.
     */
    public synchronized boolean keyUp(Emission e, ChainBudget b, UUID source, @Nullable Pose pose) {
        boolean newBurst = e.startMicros() > lastTxEndMicros + BURST_GAP_MICROS;
        lastTxEndMicros = Math.max(lastTxEndMicros, e.endMicros());
        if (newBurst) {
            bursts++;
            burstCancelled = NeoForge.EVENT_BUS.post(new RadioTransmitEvent(source, pose, e.channel(),
                    AmpModel.wToDbm(b.forwardW()), "amplifier")).isCanceled();
        }
        return !burstCancelled;
    }

    /** Heat (W averaged over this tick) and FE an exciter's transmission put through this amplifier this tick. */
    void deposit(double heatW, double fe, ChainBudget budget, @Nullable BlockPos feed) {
        pendingHeatW += heatW;
        pendingFe += fe;
        lastBudget = budget;
        lastFeed = feed;
        if (level != null) lastTxTick = level.getGameTime();
    }

    /** An insulator arced: hold output under {@code limitW} for a while (protected tiers only). */
    public void arcFoldback(double limitW, long now) {
        if (!tier.protectedOutput) return;
        arcLimitW = Math.min(arcLimitW, Math.max(0, limitW));
        arcUntil = now + ARC_FOLDBACK_TICKS;
    }

    public static void serverTick(Level level, BlockPos pos, BlockState state, AmplifierBlockEntity be) {
        be.tick((ServerLevel) level, pos);
    }

    private void tick(ServerLevel level, BlockPos pos) {
        long now = level.getGameTime();
        double wpf = RadioConfig.wattsPerFePerTick();
        // Supply for the next transmissions: what the buffer (before this tick's draw) gives a full-power tick.
        ChainBudget b = lastBudget;
        double drive = b == null || b.amp() == null ? AmpModel.FULL_DRIVE_W : b.amp().driveW();
        supply = AmpModel.supplyFraction(energy.getEnergyStored(), AmpModel.targetW(tier, drive), wpf);
        // FE drawn for this tick's transmissions (fractions carried over).
        feCarry += pendingFe;
        int whole = (int) Math.floor(feCarry);
        if (whole > 0) {
            int got = energy.consume(whole);
            feDrawnTotal += got;
            feCarry -= whole;
            if (got < whole) feCarry = 0;
        }
        lastFePerTick = pendingFe;
        pendingFe = 0;
        if (now >= arcUntil) arcLimitW = Double.POSITIVE_INFINITY;
        // Heat.
        double load = AmpModel.ampLoad(tier, pendingHeatW);
        pendingHeatW = 0;
        theta = ThermalModel.Part.AMPLIFIER.step(theta, load);
        boolean tx = now - lastTxTick <= 1;
        if (tx && b != null && b.amp() != null) {
            boolean badSwr = b.amp().foldback() || b.rhoAtAmp() > AmpModel.rho(3);
            if (badSwr && now % 10 == 0) {
                HazardActions.burst(level, pos, ParticleTypes.SMOKE, 2);
                if (!warnedSwr) {
                    warnedSwr = true;
                    com.example.evanscomputermod.EvansComputerMod.LOGGER.warn("[radio] {} at {}: high SWR {} ({} reflected){}",
                            tier.id, pos.toShortString(), String.format(Locale.ROOT, "%.1f", b.swrAtAmp()),
                            w(b.reflectedW()), b.amp().foldback() ? ", folding back to " + w(b.amp().outW()) : ", no protection");
                }
            }
            if (!badSwr) warnedSwr = false;
        }
        if (theta >= ThermalModel.WARNING && now % 10 == 0) {
            HazardActions.burst(level, pos, ParticleTypes.LARGE_SMOKE, 3);
            HazardActions.sound(level, pos, SoundEvents.FIRE_EXTINGUISH, 0.3f, 1.4f);
        }
        if (theta >= 1 && now % 20 == 0) {
            String detail = tier.id + " burnt out: " + (b == null ? "" : String.format(Locale.ROOT, "SWR %.1f, %s reflected", b.swrAtAmp(), w(b.reflectedW())));
            BlockPos feed = lastFeed == null ? pos : lastFeed;
            if (HazardActions.equipmentHazards(level) && HazardActions.overload(level, feed, b == null ? 0 : b.forwardW(), tier.ratedW, pos, "swr")) {
                HazardActions.destroy(level, pos, owner, HazardEvent.Kind.AMPLIFIER_BURNOUT, detail,
                        new ItemStack(RadioHazardContent.MELTED_SCRAP.get()));
            }
            if (!isRemoved()) theta = 1;
        }
        if (now % 20 == 0) setChanged();
    }

    /** Status lines for players (right-click) and the peripheral. */
    public List<String> statusLines(@Nullable List<String> chainWarnings) {
        List<String> out = new ArrayList<>();
        ChainBudget b = lastBudget;
        boolean tx = transmitting();
        String head = String.format(Locale.ROOT, "%s: %s · %.0f °C · %d / %d FE", name(), tx && b != null && b.amp() != null
                ? "out " + w(b.amp().outW()) + " (drive " + String.format(Locale.ROOT, "%.1f W", b.amp().driveW()) + ")" : "idle",
                temperatureC(), energy.getEnergyStored(), energy.getMaxEnergyStored());
        out.add(head);
        if (b != null && b.amp() != null) {
            out.add(String.format(Locale.ROOT, "SWR %s · reflected %s · %s FE/t while transmitting%s%s", ratio(b.swrAtAmp()), w(b.reflectedW()),
                    String.format(Locale.ROOT, "%.0f", AmpModel.feDrawPerTick(b.amp().amplifiedW(), 1, RadioConfig.wattsPerFePerTick())),
                    supply < 1 ? String.format(Locale.ROOT, " · BROWNOUT %.0f%% supply", supply * 100) : "",
                    b.amp().foldback() ? " · FOLDBACK" : ""));
            if (!tier.protectedOutput && b.reflectedW() > 0) {
                double t = ThermalModel.Part.AMPLIFIER.timeToFailure(theta, AmpModel.ampLoad(tier, b.amp().heatW()));
                if (Double.isFinite(t)) out.add("Unprotected: burns out after " + ThermalModel.describeSeconds(t) + " more at this SWR");
            }
        }
        if (chainWarnings != null) out.addAll(chainWarnings);
        return out;
    }

    public Map<String, Object> status() {
        Map<String, Object> m = new LinkedHashMap<>();
        ChainBudget b = lastBudget;
        boolean tx = transmitting() && b != null && b.amp() != null;
        m.put("tier_w", tier.ratedW);
        m.put("transmitting", tx);
        m.put("output_w", tx ? b.amp().outW() : 0.0);
        m.put("drive_w", b == null || b.amp() == null ? 0.0 : b.amp().driveW());
        m.put("reflected_w", tx ? b.reflectedW() : 0.0);
        m.put("swr", b == null ? 1.0 : Math.min(999, b.swrAtAmp()));
        m.put("foldback", b != null && b.amp() != null && b.amp().foldback());
        m.put("temperature_c", temperatureC());
        m.put("fe_per_tick", lastFePerTick);
        m.put("energy", energy.getEnergyStored());
        m.put("capacity", energy.getMaxEnergyStored());
        m.put("supply", supply);
        m.put("antenna_w", tx ? b.acceptedW() : 0.0);
        m.put("bursts", bursts);
        return m;
    }

    private String name() {
        return switch (tier) {
            case W100 -> "Amplifier 100 W";
            case KW1 -> "Amplifier 1 kW";
            case KW10 -> "Amplifier 10 kW";
        };
    }

    static String w(double watts) {
        return watts >= 1000 ? String.format(Locale.ROOT, "%.2f kW", watts / 1000) : String.format(Locale.ROOT, "%.1f W", watts);
    }

    static String ratio(double swr) {
        return Double.isFinite(swr) && swr < 100 ? String.format(Locale.ROOT, "%.1f:1", swr) : "∞";
    }

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        energy.save(tag, "Energy");
        tag.putDouble("Theta", theta);
        if (owner != null) tag.putUUID("Owner", owner);
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        energy.load(tag, "Energy");
        theta = tag.getDouble("Theta");
        owner = tag.hasUUID("Owner") ? tag.getUUID("Owner") : null;
    }
}
//?}
