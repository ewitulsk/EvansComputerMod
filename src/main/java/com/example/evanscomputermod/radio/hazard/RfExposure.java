package com.example.evanscomputermod.radio.hazard;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.event.HazardEvent;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.sounds.SoundEvents;
import net.minecraft.sounds.SoundSource;
import net.minecraft.world.damagesource.DamageSource;
import net.minecraft.world.damagesource.DamageType;

import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

/**
 * RF fields around transmitting antennas: what the RF meter reads and what
 * RF exposure (hazard level full) is judged by. Each transmitting chain
 * reports its antenna (pose, pattern) and the power the antenna accepts every
 * tick it transmits; the far-field strength at distance d is
 * <pre>  E = √(30 · P · G) / d   (V/m)</pre>
 * Fields from several sources add in power. The exposure limit is the ICNIRP
 * occupational reference level (610/f V/m below 10 MHz, 61 V/m to 400 MHz,
 * 3√f above, 137 V/m from 2 GHz); above it a player takes damage over time,
 * only while something transmits.
 */
public final class RfExposure {
    public static final ResourceKey<DamageType> RF_BURN = ResourceKey.create(Registries.DAMAGE_TYPE, EvansComputerMod.id("rf_burn"));
    public static final int CHECK_TICKS = 10;

    /** A transmitting antenna. {@code powerW} is the power it accepts (its gain includes its efficiency). */
    public record Source(String dimension, Pose pose, AntennaPattern pattern, double powerW, double hz, long untilTick) {}

    /** Field at a point: total V/m, the strongest source's frequency, and the power a 0 dBi antenna would pick up. */
    public record Field(double vPerM, double hz, double dbmIsotropic) {
        public static final Field NONE = new Field(0, 0, Double.NEGATIVE_INFINITY);
    }

    private static final Map<Object, Source> SOURCES = new ConcurrentHashMap<>();

    private RfExposure() {}

    public static void update(Object key, Source s) {
        SOURCES.put(key, s);
    }

    public static void remove(Object key) {
        SOURCES.remove(key);
    }

    /** ICNIRP occupational electric-field reference level at {@code hz}, V/m. */
    public static double limitVPerM(double hz) {
        double mhz = hz / 1e6;
        if (mhz < 1) return 610;
        if (mhz < 10) return 610 / mhz;
        if (mhz < 400) return 61;
        if (mhz < 2000) return 3 * Math.sqrt(mhz);
        return 137;
    }

    /** The far-field strength of one source at a distance in a local direction. */
    public static double fieldVPerM(double powerW, double gainDbi, double distanceM) {
        double g = Math.pow(10, gainDbi / 10);
        return Math.sqrt(30 * Math.max(0, powerW) * g) / Math.max(0.5, distanceM);
    }

    public static Field at(String dimension, double x, double y, double z, long now) {
        double sum = 0, best = 0, bestHz = 0;
        for (Source s : SOURCES.values()) {
            if (s.untilTick < now || !s.dimension.equals(dimension) || s.powerW <= 0) continue;
            double dx = x - s.pose.x(), dy = y - s.pose.y(), dz = z - s.pose.z();
            double d = Math.sqrt(dx * dx + dy * dy + dz * dz);
            double gain;
            if (d < 1e-6) gain = s.pattern.peakGainDbi();
            else {
                double[] l = s.pose.toLocal(dx / d, dy / d, dz / d);
                gain = s.pattern.gainDbi(l[0], l[1], l[2]);
            }
            double e = fieldVPerM(s.powerW, gain, d);
            sum += e * e;
            if (e > best) {
                best = e;
                bestHz = s.hz;
            }
        }
        if (sum <= 0) return Field.NONE;
        double e = Math.sqrt(sum);
        // Power density S = E²/377; a 0 dBi antenna's aperture is λ²/4π.
        double lambda = 299_792_458.0 / Math.max(1, bestHz);
        double pw = e * e / 376.73 * lambda * lambda / (4 * Math.PI);
        return new Field(e, bestHz, 10 * Math.log10(pw * 1000));
    }

    static void tick(MinecraftServer server) {
        if (server.getTickCount() % CHECK_TICKS != 0) return;
        long now = server.overworld().getGameTime();
        SOURCES.values().removeIf(s -> s.untilTick < now - 200);
        if (SOURCES.isEmpty()) return;
        for (ServerPlayer p : server.getPlayerList().getPlayers()) {
            ServerLevel level = p.serverLevel();
            Field f = at(level.dimension().location().toString(), p.getX(), p.getEyeY(), p.getZ(), level.getGameTime());
            if (f.vPerM <= 0) continue;
            double limit = limitVPerM(f.hz);
            if (f.vPerM >= 0.5 * limit)
                level.playSound(null, p.blockPosition(), SoundEvents.BEACON_AMBIENT, SoundSource.AMBIENT, 0.3f, 1.8f);
            if (f.vPerM < limit || !HazardActions.fullHazards(level, p.blockPosition()) || p.isCreative() || p.isSpectator()) continue;
            String detail = String.format(java.util.Locale.ROOT, "%.0f V/m at %.2f MHz (limit %.0f V/m)", f.vPerM, f.hz / 1e6, limit);
            if (!HazardActions.announce(level, p.blockPosition(), HazardEvent.Kind.RF_EXPOSURE, p, detail)) continue;
            DamageSource src = new DamageSource(level.registryAccess().registryOrThrow(Registries.DAMAGE_TYPE).getHolderOrThrow(RF_BURN));
            p.hurt(src, (float) Math.min(4, f.vPerM / limit));
        }
    }
}
//?}
