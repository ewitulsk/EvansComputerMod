package com.example.evanscomputermod.radio.cellular;

import com.example.evanscomputermod.radio.framed.PhyProfile;
import com.example.evanscomputermod.radio.api.AntennaPattern;

import java.util.List;

/**
 * Antenna patterns for cell-site panels and modems, pure. A site's endpoint has an
 * identity orientation (its local frame is the world frame: +X east, +Y up,
 * +Z south), and its pattern is the best of the panels fitted to its mast, each
 * pointing out horizontally along its facing.
 *
 * <ul>
 *   <li>{@link #sector}: an LTE sector panel, the 3GPP TR 36.814 parabolic model
 *       (65° horizontal / 20° vertical half-power beamwidth, 25 dB front-to-back,
 *       3° electrical downtilt).</li>
 *   <li>{@link #steered}: a mmWave phased array whose beam is steered at each
 *       device: the full array gain anywhere in its ±60° x ±45° scan range, less
 *       the cos<sup>1.5</sup> scan loss, and the back lobe outside it.</li>
 * </ul>
 */
public final class CellAntennas {
    private CellAntennas() {}

    private static final double[] VERTICAL = {0, 1, 0};

    /** One panel's gain towards a direction given in the panel's frame (boresight +Z), dBi. */
    public interface Panel {
        double gainDbi(double az, double el);

        double peakDbi();
    }

    /** 3GPP sector panel. */
    public static Panel sector(double peakDbi, double hBeamDeg, double vBeamDeg, double frontBackDb, double downtiltDeg) {
        return new Panel() {
            @Override public double gainDbi(double az, double el) {
                double azDeg = Math.toDegrees(az), elDeg = Math.toDegrees(el) + downtiltDeg;
                double ah = Math.min(12 * Math.pow(azDeg / hBeamDeg, 2), frontBackDb);
                double av = Math.min(12 * Math.pow(elDeg / vBeamDeg, 2), frontBackDb - 5);
                return peakDbi - Math.min(ah + av, frontBackDb);
            }

            @Override public double peakDbi() {
                return peakDbi;
            }
        };
    }

    /** Electronically steered array. */
    public static Panel steered(double peakDbi, double scanAzDeg, double scanElDeg, double backDb) {
        return new Panel() {
            @Override public double gainDbi(double az, double el) {
                if (Math.abs(Math.toDegrees(az)) > scanAzDeg || Math.abs(Math.toDegrees(el)) > scanElDeg) return peakDbi - backDb;
                double c = Math.cos(az) * Math.cos(el);
                return peakDbi + 15 * Math.log10(Math.max(0.05, c));
            }

            @Override public double peakDbi() {
                return peakDbi;
            }
        };
    }

    /** The panel kind each technology's tower antenna block carries. */
    public static Panel panelFor(PhyProfile tech) {
        return switch (tech) {
            case LTE -> sector(tech.towerPanelGainDbi, 65, 20, 25, 3);
            case MMWAVE -> steered(tech.towerPanelGainDbi, 60, 45, 25);
        };
    }

    /**
     * A tower: panels pointing along the given horizontal unit vectors (x, z) in
     * world space, with {@code feedLossDb} of feeder between radio and antennas.
     * No panels = a dead antenna (-100 dBi).
     */
    public static AntennaPattern tower(Panel panel, List<double[]> boresights, double feedLossDb) {
        List<double[]> dirs = List.copyOf(boresights);
        return new AntennaPattern() {
            @Override public double gainDbi(double lx, double ly, double lz) {
                if (dirs.isEmpty()) return -100;
                double h = Math.sqrt(lx * lx + lz * lz);
                double el = Math.atan2(ly, h);
                double best = Double.NEGATIVE_INFINITY;
                for (double[] b : dirs) {
                    double dot = lx * b[0] + lz * b[1], cross = lx * b[1] - lz * b[0];
                    double az = h < 1e-9 ? 0 : Math.atan2(cross, dot);
                    best = Math.max(best, panel.gainDbi(az, el));
                }
                return best;
            }

            @Override public double[] polarization(double lx, double ly, double lz) {
                return VERTICAL;
            }

            @Override public double peakGainDbi() {
                return dirs.isEmpty() ? -100 : panel.peakDbi();
            }

            @Override public double feedLossDb() {
                return feedLossDb;
            }
        };
    }

    /**
     * SWR of a tower panel at {@code hz}: 1.3 inside its band (LTE panels cover 698-960 MHz, mmWave arrays
     * 26.5-29.5 GHz, like real ones), 100 outside it (a panel is no antenna for other bands).
     */
    public static double panelSwr(PhyProfile tech, double hz) {
        boolean in = tech == PhyProfile.LTE ? hz >= 698e6 && hz <= 960e6 : hz >= 26.5e9 && hz <= 29.5e9;
        return in ? 1.3 : 100;
    }

    /** A modem's antenna: LTE a half-wave whip, mmWave its array steered at the tower (constant gain). */
    public static AntennaPattern modem(PhyProfile tech) {
        if (tech == PhyProfile.LTE) return AntennaPattern.VERTICAL_DIPOLE;
        double g = tech.ueAntennaGainDbi;
        return new AntennaPattern() {
            @Override public double gainDbi(double lx, double ly, double lz) { return g; }
            @Override public double[] polarization(double lx, double ly, double lz) { return VERTICAL; }
            @Override public double peakGainDbi() { return g; }
        };
    }
}
