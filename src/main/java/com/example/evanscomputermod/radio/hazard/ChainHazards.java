package com.example.evanscomputermod.radio.hazard;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.amp.AmplifierBlockEntity;
import com.example.evanscomputermod.radio.amp.ChainBudget;
import com.example.evanscomputermod.radio.amp.TransmitChain;
import com.example.evanscomputermod.radio.antenna.Antenna;
import com.example.evanscomputermod.radio.antenna.graph.AntennaGraph;
import com.example.evanscomputermod.radio.api.event.HazardEvent;
import com.example.evanscomputermod.radio.conductor.ConductorBlock;
import net.minecraft.core.BlockPos;
import net.minecraft.core.particles.DustParticleOptions;
import net.minecraft.core.particles.ParticleTypes;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.sounds.SoundEvents;
import net.minecraft.world.item.ItemStack;
import org.joml.Vector3f;
import org.jetbrains.annotations.Nullable;

import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.UUID;

/**
 * The thermal state of one transmit chain's passive parts: the antenna's
 * hottest wire segment, its weakest insulator (or bare end), and each
 * feedline run. Integrated once per server tick by the chain's exciter
 * (heat only flows while transmitting, scaled by airtime); warning stages
 * at {@link ThermalModel#WARNING}; consequences at the failure point:
 * <ul>
 *   <li>wire: the hottest segment melts and drops scrap;</li>
 *   <li>insulator: an arc (amplifier foldback; fire only at hazard level full
 *       with a flammable block within {@link #ARC_RADIUS});</li>
 *   <li>coax: the first block of the overloaded run melts.</li>
 * </ul>
 * Each failure posts an {@code AntennaOverloadEvent} first; world edits then
 * go through {@link HazardActions}. A prevented failure (cancelled, hazard
 * level off, claim) keeps its warning and is retried every
 * {@link #RETRY_TICKS}.
 */
public final class ChainHazards {
    public static final int ARC_RADIUS = 2;
    public static final int RETRY_TICKS = 20;
    public static final int WARN_EVERY_TICKS = 10;
    private static final DustParticleOptions GLOW = new DustParticleOptions(new Vector3f(1.0f, 0.45f, 0.1f), 1.2f);

    private double wire, insulator;
    private double[] coax = new double[0];
    private long retryWire, retryInsulator, retryCoax;
    @Nullable private BlockPos feed;
    private volatile List<String> status = List.of();
    private volatile String lastEvent = "";

    /** Normalized temperatures, for tests and the amplifier status. */
    public double wireTheta() { return wire; }
    public double insulatorTheta() { return insulator; }
    public double coaxTheta(int hop) { return hop < coax.length ? coax[hop] : 0; }

    /** Warning lines (empty when everything is cool). */
    public List<String> status() { return status; }

    /** What the last failure (or prevented failure) was. */
    public String lastEvent() { return lastEvent; }

    /**
     * One tick.
     *
     * @param budget the budget of the transmissions this tick (null: none yet)
     * @param duty   airtime fraction of this tick
     * @param owner  who built the antenna (fake-player owner for edits)
     */
    public void tick(ServerLevel level, TransmitChain chain, @Nullable Antenna antenna, @Nullable ChainBudget budget,
                     double duty, @Nullable AmplifierBlockEntity amp, @Nullable UUID owner) {
        BlockPos f = chain.feed();
        if (f == null ? feed != null : !f.equals(feed)) {
            wire = insulator = 0;
            feed = f;
        }
        if (coax.length != chain.hops().size()) coax = new double[chain.hops().size()];
        double hz = budget == null ? 0 : budget.hz();
        double accepted = budget == null ? 0 : budget.acceptedW() * duty;
        boolean present = antenna != null && antenna.present() && f != null;
        double wireLoad = present && antenna.wireLimitW() > 0 ? accepted / antenna.wireLimitW() : 0;
        double vLimit = present ? antenna.voltageLimitW() : Double.NaN;
        double insLoad = present && vLimit > 0 && Double.isFinite(vLimit) ? accepted / vLimit : 0;
        wire = ThermalModel.Part.WIRE.step(wire, wireLoad);
        insulator = ThermalModel.Part.INSULATOR.step(insulator, insLoad);
        for (int j = 0; j < coax.length; j++) {
            var line = chain.hops().get(j).line();
            double load = 0;
            if (budget != null && line.length() > 0 && j < budget.lineInW().length)
                load = budget.lineInW()[j] * duty / line.powerRatingW(hz);
            coax[j] = ThermalModel.Part.COAX.step(coax[j], load);
        }

        long now = level.getGameTime();
        List<String> lines = new ArrayList<>();
        boolean warnTick = now % WARN_EVERY_TICKS == 0;
        if (present) {
            BlockPos hot = wireAt(level, antenna.report().wireLimitAt(), f);
            if (wire >= ThermalModel.WARNING) {
                lines.add(line(antenna.report().wireLimitLabel(), ThermalModel.Part.WIRE, wire, wireLoad, "melts"));
                if (warnTick) {
                    level.sendParticles(GLOW, hot.getX() + 0.5, hot.getY() + 0.5, hot.getZ() + 0.5, 6, 0.3, 0.1, 0.3, 0);
                    HazardActions.sound(level, hot, SoundEvents.FIRE_EXTINGUISH, 0.25f, 1.6f);
                }
            }
            if (wire >= 1 && now >= retryWire) {
                retryWire = now + RETRY_TICKS;
                if (melt(level, f, hot, accepted / Math.max(duty, 1e-9), antenna.wireLimitW(), "wire_current", owner,
                        antenna.report().wireLimitLabel() + " melted at " + watts(budget) + " (rated " + w(antenna.wireLimitW()) + ")"))
                    wire = 0;
                else wire = 1;
            }
            BlockPos arcAt = at(antenna.report().voltageLimitAt(), f);
            if (insulator >= ThermalModel.WARNING) {
                lines.add(line(antenna.report().voltageLimitLabel(), ThermalModel.Part.INSULATOR, insulator, insLoad, "arcs"));
                if (warnTick) {
                    HazardActions.burst(level, arcAt, ParticleTypes.ELECTRIC_SPARK, 4);
                    HazardActions.sound(level, arcAt, SoundEvents.FIRE_AMBIENT, 0.4f, 2.0f);
                }
            }
            if (insulator >= 1 && now >= retryInsulator) {
                retryInsulator = now + RETRY_TICKS;
                insulator = 1;
                double p = accepted / Math.max(duty, 1e-9);
                if (HazardActions.equipmentHazards(level, arcAt) && HazardActions.overload(level, f, p, vLimit, arcAt, "insulator_voltage")) {
                    String what = antenna.report().voltageLimitLabel() + " arced at " + w(p) + " (rated " + w(vLimit) + ")";
                    boolean fire = HazardActions.arc(level, arcAt, ARC_RADIUS, owner, what);
                    // Foldback: protected amplifiers hold the antenna under 80% of the voltage limit for a while.
                    if (amp != null && budget != null && budget.forwardW() > 0 && p > 0)
                        amp.arcFoldback(0.8 * vLimit * budget.forwardW() / p, now);
                    lastEvent = what + (fire ? "; set a fire" : "");
                    insulator = 0.5;
                } else lastEvent = "arc prevented at " + arcAt;
            }
        }
        for (int j = 0; j < coax.length; j++) {
            var line = chain.hops().get(j).line();
            if (line.length() == 0) continue;
            BlockPos first = line.blocks().get(0);
            if (coax[j] >= ThermalModel.WARNING) {
                lines.add(String.format(Locale.ROOT, "coax run %d at %s: %.0f °C", j + 1, first.toShortString(),
                        ThermalModel.Part.COAX.celsius(coax[j])));
                if (warnTick) HazardActions.burst(level, first, ParticleTypes.SMOKE, 3);
            }
            if (coax[j] >= 1 && now >= retryCoax) {
                retryCoax = now + RETRY_TICKS;
                double p = budget == null ? 0 : budget.lineInW()[j];
                BlockPos fp = f == null ? first : f;
                if (melt(level, fp, first, p, line.powerRatingW(hz), "coax_heat", owner, "coax melted at " + w(p)
                        + " (rated " + w(line.powerRatingW(hz)) + ")"))
                    coax[j] = 0;
                else coax[j] = 1;
            }
        }
        status = List.copyOf(lines);
    }

    private boolean melt(ServerLevel level, BlockPos feedPos, BlockPos at, double powerW, double ratedW, String cause,
                         @Nullable UUID owner, String detail) {
        if (!HazardActions.equipmentHazards(level, at)) {
            lastEvent = "hazards off: " + detail.replace("melted", "would melt");
            return false;
        }
        if (!(level.getBlockState(at).getBlock() instanceof ConductorBlock)) {
            lastEvent = "nothing to melt at " + at.toShortString();
            return false;
        }
        if (!HazardActions.overload(level, feedPos, powerW, ratedW, at, cause)) {
            lastEvent = "overload cancelled: " + detail;
            return false;
        }
        boolean done = HazardActions.destroy(level, at, owner, HazardEvent.Kind.MELT, detail,
                new ItemStack(RadioHazardContent.MELTED_SCRAP.get()));
        lastEvent = done ? detail : "melt prevented: " + detail;
        return done;
    }

    /** The wire block holding the hottest segment's midpoint (a neighbour when it falls inside the feed point). */
    private static BlockPos wireAt(ServerLevel level, @Nullable AntennaGraph.Point p, BlockPos fallback) {
        BlockPos c = at(p, fallback);
        if (isWire(level, c) || p == null) return c;
        BlockPos best = c;
        double bestD = Double.MAX_VALUE;
        for (BlockPos n : BlockPos.withinManhattan(c, 1, 1, 1)) {
            if (!isWire(level, n)) continue;
            double d = n.getCenter().distanceToSqr(p.x(), p.y(), p.z());
            if (d < bestD) {
                bestD = d;
                best = n.immutable();
            }
        }
        return best;
    }

    private static boolean isWire(ServerLevel level, BlockPos pos) {
        return level.getBlockState(pos).getBlock() instanceof ConductorBlock c && c.role() == ConductorBlock.Role.CONDUCTOR;
    }

    private static BlockPos at(@Nullable AntennaGraph.Point p, BlockPos fallback) {
        return p == null ? fallback : BlockPos.containing(p.x(), p.y(), p.z());
    }

    private static String line(String what, ThermalModel.Part part, double theta, double load, String fails) {
        double t = part.timeToFailure(theta, load);
        String temp = part.failRiseK > 0 ? String.format(Locale.ROOT, "%.0f °C", part.celsius(theta))
                : String.format(Locale.ROOT, "%.0f%% of rating", 100 * theta);
        return what + ": " + temp + (Double.isInfinite(t) ? "" : ", " + fails + " in " + ThermalModel.describeSeconds(t));
    }

    private static String watts(@Nullable ChainBudget b) {
        return b == null ? "?" : w(b.acceptedW());
    }

    static String w(double watts) {
        return watts >= 1000 ? String.format(Locale.ROOT, "%.1f kW", watts / 1000) : String.format(Locale.ROOT, "%.0f W", watts);
    }
}
//?}
