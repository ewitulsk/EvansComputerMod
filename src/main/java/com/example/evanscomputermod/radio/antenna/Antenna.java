package com.example.evanscomputermod.radio.antenna;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.antenna.graph.AntennaGraph;
import com.example.evanscomputermod.radio.antenna.graph.AntennaReport;
import com.example.evanscomputermod.radio.antenna.solver.Complex;
import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.sensor.SensorSable;
import net.minecraft.core.BlockPos;
import net.minecraft.resources.ResourceKey;
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.Vec3;
import org.jetbrains.annotations.Nullable;

/**
 * A built antenna as other lanes see it: an immutable snapshot of the cached
 * analysis of the conductor graph on one feed point. Get one with
 * {@link AntennaManager#get} (never blocks; an estimate while the solve is
 * pending) or {@link AntennaManager#whenSolved}.
 *
 * <p>For a {@code RadioEndpoint}: {@link #pattern(double)} for
 * {@code antenna()}, {@link #pose(Level)} for {@code pose()}. For an
 * amplifier: {@link #swrAt}, {@link #powerLimitW}, {@link #weakestLink}. For
 * hazards: {@link #peakCurrentA} and {@link #peakEndVoltageV} at a power.
 */
public record Antenna(ResourceKey<Level> dimension, BlockPos feed, AntennaReport report, @Nullable AntennaGraph graph,
        boolean pending, long version) {

    /** True when a conductor is attached (solved or estimated). */
    public boolean present() { return report.present(); }

    public boolean solved() { return report.solved() && !pending; }

    /** Lowest series resonance, Hz (NaN if none in range or no antenna). */
    public double resonantHz() { return report.resonantHz(); }

    /** Feed impedance at the analysis frequency (resonance), Ω. */
    @Nullable
    public Complex feedImpedance() { return report.feedImpedance(); }

    /** Feed impedance at {@code hz} (interpolated from the cached sweeps), Ω; null outside the swept bands or with no antenna. */
    @Nullable
    public Complex impedanceAt(double hz) { return present() ? report.impedanceAt(hz) : null; }

    /** SWR against 50 Ω at {@code hz}; +∞ outside the swept bands or with no antenna. */
    public double swrAt(double hz) { return present() ? report.swrAt(hz) : Double.POSITIVE_INFINITY; }

    public double swrAt(double hz, double z0) { return present() ? report.swrAt(hz, z0) : Double.POSITIVE_INFINITY; }

    /** Radiation efficiency at {@code hz}, 0..1. */
    public double efficiencyAt(double hz) { return present() ? report.efficiencyAt(hz) : 0; }

    /** 2:1 SWR band around resonance as {low, high} Hz, or null. */
    @Nullable
    public double[] swrBand() {
        return Double.isFinite(report.swrBandLowHz()) ? new double[] {report.swrBandLowHz(), report.swrBandHighHz()} : null;
    }

    /** Gain pattern to use at {@code hz}, in the antenna's local (structure) frame; includes mismatch loss as feedLossDb. */
    public AntennaPattern pattern(double hz) { return report.patternAt(hz); }

    /** Continuous power the antenna takes before its weakest part fails, W (0 = no antenna). */
    public double powerLimitW() { return report.powerLimitW(); }

    /** Power at which the wires reach their current rating, W. */
    public double wireLimitW() { return report.wireLimitW(); }

    /** Power at which insulators / bare ends / the feed reach their voltage rating, W (NaN for estimates). */
    public double voltageLimitW() { return report.voltageLimitW(); }

    /** Name of the part that sets {@link #powerLimitW}, e.g. "copper wire" or "insulators". */
    public String weakestLink() { return report.weakestLink(); }

    /** Cause of {@link #powerLimitW} as {@code AntennaOverloadEvent} names it: "wire_current" or "insulator_voltage". */
    public String limitCause() { return report.limitCause(); }

    /**
     * Block of the weakest link (the hottest wire segment, or the limiting
     * insulator / bare wire end / feed point), in the feed's own coordinates
     * (plot space on a Sable structure). The feed point when unknown.
     */
    public BlockPos weakestLinkPos() {
        var p = report.weakestLinkAt();
        return p == null ? feed : BlockPos.containing(p.x(), p.y(), p.z());
    }

    /** Peak RF current anywhere on the antenna at {@code watts}, A (amplitude; ÷√2 for RMS). */
    public double peakCurrentA(double watts) { return report.peakCurrentPerWatt() * Math.sqrt(watts); }

    /** Peak open-end voltage at {@code watts}, V. */
    public double peakEndVoltageV(double watts) { return report.peakEndVoltagePerWatt() * Math.sqrt(watts); }

    /** "Resonant at 7.1 MHz · 2:1 SWR band 6.9–7.3 MHz · rated 200 W (wire) / 1.4 kW (insulators)". */
    public String summary() { return report.summary(); }

    public String details() {
        String d = report.details();
        String note = graph == null ? "" : graph.truncationNote;
        if (note.isEmpty()) return d;
        return (d.isEmpty() ? "" : d + " · ") + "only part of the antenna was analysed: " + note;
    }

    /**
     * World pose of the feed point: on a Sable sub-level, the projected
     * position and the sub-level's orientation (so patterns rotate with the
     * ship); elsewhere the block centre with identity orientation.
     */
    public Pose pose(Level level) {
        Vec3 c = Vec3.atCenterOf(feed);
        String dim = dimension.location().toString();
        SensorSable.Frame f = SensorSable.frame(level, c);
        if (!f.onStructure()) return Pose.at(dim, c.x, c.y, c.z);
        Vec3 w = f.position(c);
        Vec3 x = f.normal(new Vec3(1, 0, 0)), y = f.normal(new Vec3(0, 1, 0)), z = f.normal(new Vec3(0, 0, 1));
        float[] q = quaternion(x, y, z);
        return new Pose(dim, w.x, w.y, w.z, q[0], q[1], q[2], q[3]);
    }

    /** Unit quaternion {x, y, z, w} of the rotation whose matrix columns are the images of the axes. */
    static float[] quaternion(Vec3 cx, Vec3 cy, Vec3 cz) {
        double m00 = cx.x, m10 = cx.y, m20 = cx.z, m01 = cy.x, m11 = cy.y, m21 = cy.z, m02 = cz.x, m12 = cz.y, m22 = cz.z;
        double tr = m00 + m11 + m22, qx, qy, qz, qw;
        if (tr > 0) {
            double s = Math.sqrt(tr + 1) * 2;
            qw = s / 4; qx = (m21 - m12) / s; qy = (m02 - m20) / s; qz = (m10 - m01) / s;
        } else if (m00 > m11 && m00 > m22) {
            double s = Math.sqrt(1 + m00 - m11 - m22) * 2;
            qw = (m21 - m12) / s; qx = s / 4; qy = (m01 + m10) / s; qz = (m02 + m20) / s;
        } else if (m11 > m22) {
            double s = Math.sqrt(1 + m11 - m00 - m22) * 2;
            qw = (m02 - m20) / s; qx = (m01 + m10) / s; qy = s / 4; qz = (m12 + m21) / s;
        } else {
            double s = Math.sqrt(1 + m22 - m00 - m11) * 2;
            qw = (m10 - m01) / s; qx = (m02 + m20) / s; qy = (m12 + m21) / s; qz = s / 4;
        }
        double n = Math.sqrt(qx * qx + qy * qy + qz * qz + qw * qw);
        return new float[] {(float) (qx / n), (float) (qy / n), (float) (qz / n), (float) (qw / n)};
    }
}
//?}
