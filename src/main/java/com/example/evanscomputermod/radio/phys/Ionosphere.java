package com.example.evanscomputermod.radio.phys;

/**
 * Deterministic day/night ionosphere for MF/HF skywave, driven by Minecraft
 * day time (ticks 0..24000: 0 = sunrise, 6000 = noon, 12000 = sunset, 18000 = midnight).
 *
 * <p><b>Sun.</b> The solar zenith angle &chi; follows cos &chi; = sin(2&pi;t/24000)
 * (equator at equinox).
 *
 * <p><b>F2 layer.</b> Critical frequency foF2 runs from {@code foF2NightHz} at night to
 * {@code foF2DayHz} at noon as foF2 = night + (day&minus;night)&middot;&radic;max(0, cos &chi;).
 * A mirror at virtual height {@code layerHeightM} over a spherical earth gives, for a
 * hop of ground range D (half-angle &theta; = D/2R<sub>e</sub>), the take-off elevation
 * tan &Delta; = (cos &theta; &minus; R<sub>e</sub>/(R<sub>e</sub>+h)) / sin &theta;, the incidence at the
 * layer sin &phi; = R<sub>e</sub> cos &Delta;/(R<sub>e</sub>+h), and the secant law
 * MUF = foF2 &middot; sec &phi;. Frequencies above the MUF of a hop pass through the
 * layer: that is the skip zone. Below foF2 there is no skip (NVIS).
 *
 * <p><b>D layer.</b> Daytime absorption per hop uses the George-Bradley formula
 * (ITU-R P.533 heritage, sunspot number 0):
 * L = 677.2 &middot; sec &phi;<sub>D</sub> &middot; cos(0.881&chi;)<sup>1.3</sup> / ((f<sub>MHz</sub>+f<sub>L</sub>)<sup>1.98</sup> + 10.2) dB,
 * with &phi;<sub>D</sub> the incidence at {@code dLayerHeightM}. It vanishes once
 * 0.881&chi; &ge; 90&deg; (night), so MF is absorbed by day and travels far at night.
 *
 * <p><b>Compression.</b> Real hops span thousands of km. Game distance is mapped
 * to real ground range by {@link #metresPerBlock()}, chosen so the longest single
 * hop (at {@code minElevationRad}) is {@code maxHopBlocks} (default 8000 blocks;
 * typical skip distances then come out at ~1-3k blocks). Path loss is free-space
 * loss over the slant path scaled back to blocks (1 block = 1 m), plus absorption,
 * a per-hop ionospheric loss and a ground-reflection loss between hops.
 *
 * <p>No sky (Nether, End): no skywave.
 */
public record Ionosphere(double foF2DayHz, double foF2NightHz, double layerHeightM, double dLayerHeightM,
        double minElevationRad, double maxHopBlocks, int maxHops, double ionosphericLossDbPerHop,
        double groundReflectionLossDb, double gyroFrequencyHz) {
    public static final int DAY_TICKS = 24000;
    public static final double EARTH_RADIUS_M = 6_371_000.0;
    public static final Ionosphere DEFAULT = new Ionosphere(10e6, 4e6, 300e3, 90e3, Math.toRadians(2), 8000, 6,
            1.0, 2.0, 1.0e6);

    public Ionosphere {
        if(!(foF2DayHz > 0 && foF2NightHz > 0)) throw new IllegalArgumentException("foF2 must be > 0");
        if(!(layerHeightM > dLayerHeightM && dLayerHeightM > 0)) throw new IllegalArgumentException("bad layer heights");
        if(!(maxHopBlocks > 0) || maxHops < 1) throw new IllegalArgumentException("bad hop limits");
    }

    /** Copy with a different longest-hop compression (blocks). */
    public Ionosphere withMaxHopBlocks(double blocks) {
        return new Ionosphere(foF2DayHz, foF2NightHz, layerHeightM, dLayerHeightM, minElevationRad, blocks, maxHops,
                ionosphericLossDbPerHop, groundReflectionLossDb, gyroFrequencyHz);
    }

    /** cos of the solar zenith angle at {@code dayTime} ticks (1 at noon, &minus;1 at midnight). */
    public static double cosSolarZenith(long dayTime) {
        double t = Math.floorMod(dayTime, (long) DAY_TICKS);
        return Math.sin(2.0 * Math.PI * t / DAY_TICKS);
    }

    /** Daylight fraction max(0, cos &chi;) in [0, 1]. */
    public static double daylight(long dayTime) {
        return Math.max(0, cosSolarZenith(dayTime));
    }

    /** F2 critical frequency at {@code dayTime}, Hz. */
    public double foF2Hz(long dayTime) {
        return foF2NightHz + (foF2DayHz - foF2NightHz) * Math.sqrt(daylight(dayTime));
    }

    /** Longest single-hop ground range (at the minimum elevation), real metres. */
    public double maxHopRealM() {
        double re = EARTH_RADIUS_M, rh = re + layerHeightM;
        double phi = Math.asin(re * Math.cos(minElevationRad) / rh);
        return 2.0 * re * (Math.PI / 2 - minElevationRad - phi);
    }

    /** Real metres of ground range per game block. */
    public double metresPerBlock() {
        return maxHopRealM() / maxHopBlocks;
    }

    /** Take-off elevation for a single hop of {@code hopRealM} real ground range at layer height {@code h}. */
    private static double elevationRad(double hopRealM, double h) {
        double theta = hopRealM / (2.0 * EARTH_RADIUS_M);
        if(theta <= 1e-12) return Math.PI / 2;
        return Math.atan2(Math.cos(theta) - EARTH_RADIUS_M / (EARTH_RADIUS_M + h), Math.sin(theta));
    }

    /** Incidence angle (from vertical) at height {@code h} for take-off elevation {@code elevation}. */
    private static double incidenceRad(double elevation, double h) {
        return Math.asin(Math.min(1.0, EARTH_RADIUS_M * Math.cos(elevation) / (EARTH_RADIUS_M + h)));
    }

    /** MUF for one hop of {@code hopBlocks} game blocks, Hz. */
    public double mufHz(double hopBlocks, long dayTime) {
        double el = elevationRad(Math.max(0, hopBlocks) * metresPerBlock(), layerHeightM);
        return foF2Hz(dayTime) / Math.cos(incidenceRad(el, layerHeightM));
    }

    /**
     * Skip distance for {@code freqHz}: the shortest ground range a skywave reaches, blocks.
     * 0 below foF2 (vertical incidence reflects); +infinity when even the longest hop's MUF is too low.
     */
    public double skipDistanceBlocks(double freqHz, long dayTime) {
        double fo = foF2Hz(dayTime);
        if(freqHz <= fo) return 0;
        double phi = Math.acos(fo / freqHz);
        double cosEl = (EARTH_RADIUS_M + layerHeightM) * Math.sin(phi) / EARTH_RADIUS_M;
        if(cosEl >= 1) return Double.POSITIVE_INFINITY;
        double el = Math.acos(cosEl);
        if(el < minElevationRad) return Double.POSITIVE_INFINITY;
        double theta = Math.PI / 2 - el - phi;
        return 2.0 * EARTH_RADIUS_M * theta / metresPerBlock();
    }

    /** D-layer absorption for one hop (two passes) at take-off elevation {@code elevationRad}, dB. */
    public double dLayerAbsorptionDb(double freqHz, long dayTime, double elevationRad) {
        double chi = Math.acos(Math.max(-1, Math.min(1, cosSolarZenith(dayTime))));
        double a = 0.881 * chi;
        if(a >= Math.PI / 2) return 0;
        double fm = freqHz / 1e6, fl = gyroFrequencyHz / 1e6;
        double sec = 1.0 / Math.cos(incidenceRad(elevationRad, dLayerHeightM));
        return 677.2 * sec * Math.pow(Math.cos(a), 1.3) / (Math.pow(fm + fl, 1.98) + 10.2);
    }

    /** Evaluates the skywave between two points {@code distanceBlocks} apart. */
    public SkywavePath skywave(double freqHz, double distanceBlocks, long dayTime, boolean hasSky) {
        if(!hasSky || freqHz <= 0 || !(distanceBlocks >= 0)) return SkywavePath.NONE;
        double mpb = metresPerBlock();
        double realM = distanceBlocks * mpb;
        int hops = Math.max(1, (int) Math.ceil(realM / maxHopRealM() - 1e-9));
        double skip = skipDistanceBlocks(freqHz, dayTime);
        if(hops > maxHops) return SkywavePath.none(0, skip);
        double hopM = realM / hops;
        double el = elevationRad(hopM, layerHeightM);
        double muf = foF2Hz(dayTime) / Math.cos(incidenceRad(el, layerHeightM));
        if(freqHz > muf) return SkywavePath.none(muf, skip);
        double theta = hopM / (2.0 * EARTH_RADIUS_M);
        double re = EARTH_RADIUS_M, rh = re + layerHeightM;
        double chord = Math.sqrt(re * re + rh * rh - 2.0 * re * rh * Math.cos(theta));
        double slantBlocks = hops * 2.0 * chord / mpb;
        double absorption = hops * dLayerAbsorptionDb(freqHz, dayTime, el);
        double loss = FreeSpace.lossDb(slantBlocks, freqHz) + absorption + hops * ionosphericLossDbPerHop
                + (hops - 1) * groundReflectionLossDb;
        return new SkywavePath(true, hops, loss, absorption, muf, skip, el);
    }

    /** Skywave loss in dB, or +infinity when there is no skywave. */
    public double lossDb(double freqHz, double distanceBlocks, long dayTime, boolean hasSky) {
        return skywave(freqHz, distanceBlocks, dayTime, hasSky).lossDb();
    }
}
