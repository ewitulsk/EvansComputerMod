package com.example.evanscomputermod.radio.hazard;

/**
 * The radio hazard thermal model (spec, Hazards: thermal model). Pure and
 * deterministic: no Minecraft types, no randomness.
 *
 * <p>Every part (wire segment, insulator, coax run, amplifier, tuner) is a
 * first-order thermal mass: {@code τ dθ/dt = L − θ}, where θ is the
 * normalized temperature rise (0 = ambient, 1 = the failure point) and
 * {@code L} is the load: heat in divided by the heat the part can shed
 * continuously at its rating. A part at exactly its rating approaches the
 * failure point but never reaches it; above its rating it fails after
 * <pre>  t = τ · ln((L − θ₀) / (L − 1))</pre>
 * which {@link #timeToFailure} states exactly ("fails after 12 s of
 * continuous 1 kW"). Each server tick applies the exact solution of the
 * ODE over the tick ({@link #step}), so the stepped simulation fails on tick
 * {@link #ticksToFailure} with no integration error to accumulate.
 *
 * <p>What "load" means per part (heat ∝ I²R or ∝ V², so ∝ power):
 * <ul>
 *   <li>wire: power into the antenna / the antenna's wire (current) limit;</li>
 *   <li>insulator: power into the antenna / its voltage limit (arcs, fast τ);</li>
 *   <li>coax: power entering the run / the run's power rating;</li>
 *   <li>amplifier: dissipated watts (waste heat + absorbed reflected power) / 1.25 × tier;</li>
 *   <li>tuner: mismatch power it absorbs / its dissipation rating.</li>
 * </ul>
 */
public final class ThermalModel {

    /** One server tick, seconds. */
    public static final double TICK_SECONDS = 0.05;
    /** Warning stage (glow, sizzle, corona, amplifier warning) from this normalized temperature. */
    public static final double WARNING = 0.7;
    /** Ambient temperature for display, °C. */
    public static final double AMBIENT_C = 20;

    /** A kind of part: its time constant and the temperature rise (K) at the failure point, for display. */
    public enum Part {
        WIRE("wire", 20, 180),
        INSULATOR("insulator", 2, 0),
        COAX("coax", 30, 80),
        AMPLIFIER("amplifier", 30, 110),
        TUNER("tuner", 30, 100);

        public final String id;
        public final double tauSeconds;
        public final double failRiseK;

        Part(String id, double tauSeconds, double failRiseK) {
            this.id = id;
            this.tauSeconds = tauSeconds;
            this.failRiseK = failRiseK;
        }

        /** Displayed temperature, °C, for a normalized temperature. */
        public double celsius(double theta) {
            return AMBIENT_C + theta * failRiseK;
        }

        public double step(double theta, double load) {
            return ThermalModel.step(theta, load, TICK_SECONDS, tauSeconds);
        }

        public double timeToFailure(double theta0, double load) {
            return ThermalModel.timeToFailure(theta0, load, tauSeconds);
        }

        public long ticksToFailure(double theta0, double load) {
            return ThermalModel.ticksToFailure(theta0, load, tauSeconds);
        }
    }

    private ThermalModel() {}

    /** Exact solution over {@code dt}: θ' = L + (θ − L)·e^(−dt/τ). */
    public static double step(double theta, double load, double dtSeconds, double tauSeconds) {
        double l = Math.max(0, load);
        return l + (theta - l) * Math.exp(-dtSeconds / tauSeconds);
    }

    /** Seconds from θ₀ to the failure point (θ = 1) under a constant load; +∞ if it never gets there. */
    public static double timeToFailure(double theta0, double load, double tauSeconds) {
        if (theta0 >= 1) return 0;
        if (!(load > 1)) return Double.POSITIVE_INFINITY;
        return tauSeconds * Math.log((load - theta0) / (load - 1));
    }

    /**
     * Ticks of {@link #step} from θ₀ until θ ≥ 1 under a constant load
     * (Long.MAX_VALUE if never). Matches the stepped simulation exactly: the
     * closed-form estimate is corrected against the stepped values around it.
     */
    public static long ticksToFailure(double theta0, double load, double tauSeconds) {
        double t = timeToFailure(theta0, load, tauSeconds);
        if (t == 0) return 0;
        if (Double.isInfinite(t)) return Long.MAX_VALUE;
        // Jump to a couple of ticks before the crossing in closed form, then step to it.
        long k = Math.max(0, (long) Math.ceil(t / TICK_SECONDS) - 3);
        double theta = load + (theta0 - load) * Math.exp(-k * TICK_SECONDS / tauSeconds);
        while (k > 0 && theta >= 1) {
            k--;
            theta = load + (theta0 - load) * Math.exp(-k * TICK_SECONDS / tauSeconds);
        }
        while (theta < 1) {
            theta = step(theta, load, TICK_SECONDS, tauSeconds);
            k++;
        }
        return k;
    }

    /** "12 s", "3 min 20 s", "never" for display. */
    public static String describeSeconds(double s) {
        if (Double.isInfinite(s)) return "never";
        if (s < 60) return String.format(java.util.Locale.ROOT, s < 10 ? "%.1f s" : "%.0f s", s);
        long m = (long) (s / 60);
        return m + " min " + Math.round(s - 60 * m) + " s";
    }
}
