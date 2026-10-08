package com.example.evanscomputermod.radio.antenna.graph;

import com.example.evanscomputermod.radio.antenna.solver.GainPattern;
import com.example.evanscomputermod.radio.api.AntennaPattern;

/**
 * A solved {@link GainPattern} seen through the {@link AntennaPattern}
 * contract. The local frame is the Minecraft frame of the structure the
 * antenna is built in (+X east, +Y up, +Z south); endpoints rotate it with
 * their {@code Pose} (Sable ships), so a tilting ship tilts the pattern.
 *
 * <p>{@code gainOffsetDb} shifts the whole pattern (used to read a pattern
 * solved at resonance at another frequency, scaled by that frequency's
 * efficiency) and {@code feedLossDb} carries mismatch/feedline loss.
 */
public final class SolvedPattern implements AntennaPattern {
    private final GainPattern pattern;
    private final double gainOffsetDb;
    private final double feedLossDb;

    public SolvedPattern(GainPattern pattern, double gainOffsetDb, double feedLossDb) {
        this.pattern = pattern;
        this.gainOffsetDb = gainOffsetDb;
        this.feedLossDb = feedLossDb;
    }

    public GainPattern pattern() { return pattern; }

    public SolvedPattern withOffsets(double gainOffsetDb, double feedLossDb) {
        return new SolvedPattern(pattern, gainOffsetDb, feedLossDb);
    }

    @Override
    public double gainDbi(double lx, double ly, double lz) {
        double[] s = AntennaModelBuilder.toSolver(lx, ly, lz);
        double len = Math.sqrt(s[0] * s[0] + s[1] * s[1] + s[2] * s[2]);
        if (len == 0) return peakGainDbi();
        double theta = Math.toDegrees(Math.acos(Math.max(-1, Math.min(1, s[2] / len))));
        double phi = Math.toDegrees(Math.atan2(s[1], s[0]));
        double g = pattern.gainDbi(theta, phi);
        return g <= GainPattern.FLOOR_DBI ? GainPattern.FLOOR_DBI : g + gainOffsetDb;
    }

    @Override
    public double[] polarization(double lx, double ly, double lz) {
        double[] s = AntennaModelBuilder.toSolver(lx, ly, lz);
        double len = Math.sqrt(s[0] * s[0] + s[1] * s[1] + s[2] * s[2]);
        if (len == 0) return new double[] {0, 1, 0};
        double theta = Math.acos(Math.max(-1, Math.min(1, s[2] / len))), phi = Math.atan2(s[1], s[0]);
        int ti = (int) Math.round(Math.toDegrees(theta) / GainPattern.STEP_DEG);
        int pi = (int) Math.round(Math.toDegrees(phi) / GainPattern.STEP_DEG);
        ti = Math.max(0, Math.min(GainPattern.THETA_COUNT - 1, ti));
        double fv = pattern.verticalFraction(ti, pi);
        // θ̂ = (cosθ cosφ, cosθ sinφ, −sinθ), φ̂ = (−sinφ, cosφ, 0) in the solver frame.
        double ct = Math.cos(theta), st = Math.sin(theta), cp = Math.cos(phi), sp = Math.sin(phi);
        double a = Math.sqrt(fv), b = Math.sqrt(Math.max(0, 1 - fv));
        double px = a * ct * cp - b * sp, py = a * ct * sp + b * cp, pz = -a * st;
        double n = Math.sqrt(px * px + py * py + pz * pz);
        if (n == 0) return new double[] {0, 1, 0};
        double[] m = AntennaModelBuilder.toMinecraft(px / n, py / n, pz / n);
        // Report the vertical component as positive (the sign of a linear polarization is arbitrary).
        if (m[1] < 0 || (m[1] == 0 && m[0] < 0)) { m[0] = -m[0]; m[1] = -m[1]; m[2] = -m[2]; }
        return m;
    }

    @Override
    public double peakGainDbi() { return pattern.peakDbi() + gainOffsetDb; }

    @Override
    public double feedLossDb() { return feedLossDb; }
}
