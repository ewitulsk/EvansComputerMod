package com.example.evanscomputermod.radio.phys;

/**
 * Combined path loss, a pure function of a {@link Path}. Mechanisms:
 * <ol>
 * <li><b>Free space</b> (Friis) over the 3-D distance between antennas.</li>
 * <li><b>Space wave</b>: with a {@link Ground}, the two-ray model
 *     (it tracks free space with &plusmn;6 dB lobes inside the crossover distance
 *     and falls as d<sup>4</sup> past it). When terrain diffracts, the ground
 *     reflection is not coherent, so the space wave is free space + Deygout loss.</li>
 * <li><b>Ground wave</b> (Norton): competes with the space wave; the stronger
 *     wins (it dominates at VLF-MF for low antennas).</li>
 * <li><b>Obstruction</b>: the caller's voxel-trace sum of material losses, added to every mode
 *     (walls near the antennas affect skywave too).</li>
 * <li><b>Skywave</b>: with an {@link Ionosphere}, the skywave competes with the terrestrial path.</li>
 * </ol>
 * The result is the lowest-loss mode. Only the result record is allocated.
 */
public final class PathLossModel {
    public enum Mode { FREE_SPACE, TWO_RAY, GROUND_WAVE, DIFFRACTION, SKYWAVE }

    /**
     * Inputs. {@code distanceM} is the horizontal ground distance; heights are
     * above local ground (absolute heights come from {@code terrain} when given).
     * {@code ground} null means no reflecting ground (e.g. both ends airborne);
     * {@code terrain} null means unobstructed; {@code ionosphere} null disables skywave.
     */
    public record Path(double distanceM, double freqHz, double txHeightM, double rxHeightM, Ground ground,
            Polarization polarization, TerrainProfile terrain, double obstructionDb, Ionosphere ionosphere,
            long dayTime, boolean hasSky) {

        public static Path of(double distanceM, double freqHz) {
            return new Path(distanceM, freqHz, 0, 0, null, Polarization.VERTICAL, null, 0, null, 6000, true);
        }

        public Path withHeights(double txHeightM, double rxHeightM) {
            return new Path(distanceM, freqHz, txHeightM, rxHeightM, ground, polarization, terrain, obstructionDb,
                    ionosphere, dayTime, hasSky);
        }

        public Path withGround(Ground g, Polarization pol) {
            return new Path(distanceM, freqHz, txHeightM, rxHeightM, g, pol, terrain, obstructionDb, ionosphere,
                    dayTime, hasSky);
        }

        public Path withTerrain(TerrainProfile t) {
            return new Path(distanceM, freqHz, txHeightM, rxHeightM, ground, polarization, t, obstructionDb,
                    ionosphere, dayTime, hasSky);
        }

        public Path withObstructionDb(double db) {
            return new Path(distanceM, freqHz, txHeightM, rxHeightM, ground, polarization, terrain, db, ionosphere,
                    dayTime, hasSky);
        }

        public Path withSkywave(Ionosphere iono, long dayTime, boolean hasSky) {
            return new Path(distanceM, freqHz, txHeightM, rxHeightM, ground, polarization, terrain, obstructionDb,
                    iono, dayTime, hasSky);
        }
    }

    /**
     * Output. {@code totalDb} includes obstruction and (for terrestrial modes) diffraction.
     * {@code groundExcessDb} is the chosen terrestrial base loss minus free space
     * (negative when ground reflection adds up constructively).
     * {@code skywaveDb} is the skywave loss before obstruction (+infinity when none).
     */
    public record Result(double totalDb, double freeSpaceDb, double groundExcessDb, double diffractionDb,
            double obstructionDb, double skywaveDb, Mode mode) {}

    private PathLossModel() {}

    public static double lossDb(Path p) {
        return evaluate(p).totalDb();
    }

    public static Result evaluate(Path p) {
        double f = p.freqHz();
        double d = Math.max(0, p.distanceM());
        double obs = Math.max(0, p.obstructionDb());
        double rise = p.txHeightM() - p.rxHeightM();
        if(p.terrain() != null) {
            double[] h = p.terrain().heightsM();
            rise = (h[0] + p.txHeightM()) - (h[h.length - 1] + p.rxHeightM());
        }
        double fs = FreeSpace.lossDb(Math.hypot(d, rise), f);
        double diff = p.terrain() == null ? 0 : Deygout.lossDb(p.terrain(), f, p.txHeightM(), p.rxHeightM());

        double base;
        Mode mode;
        if(diff > 0) {
            base = fs + diff;
            mode = Mode.DIFFRACTION;
        } else if(p.ground() != null) {
            base = TwoRay.lossDb(d, f, p.txHeightM(), p.rxHeightM(), p.ground(), p.polarization());
            mode = Mode.TWO_RAY;
        } else {
            base = fs;
            mode = Mode.FREE_SPACE;
        }
        if(p.ground() != null) {
            double gw = GroundWave.lossDb(d, f, p.ground(), p.polarization());
            if(gw < base) {
                base = gw;
                mode = Mode.GROUND_WAVE;
                diff = 0;
            }
        }
        double terrestrial = base + obs;
        double sky = Double.POSITIVE_INFINITY;
        if(p.ionosphere() != null) sky = p.ionosphere().lossDb(f, d, p.dayTime(), p.hasSky());
        if(sky + obs < terrestrial)
            return new Result(sky + obs, fs, 0, 0, obs, sky, Mode.SKYWAVE);
        double excess = (mode == Mode.DIFFRACTION ? fs : base) - fs;
        return new Result(terrestrial, fs, excess, diff, obs, sky, mode);
    }
}
