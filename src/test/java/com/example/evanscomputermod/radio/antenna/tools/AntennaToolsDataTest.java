package com.example.evanscomputermod.radio.antenna.tools;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.radio.antenna.graph.AntennaAnalysis;
import com.example.evanscomputermod.radio.antenna.graph.AntennaGraph;
import com.example.evanscomputermod.radio.antenna.graph.AntennaReport;
import com.example.evanscomputermod.radio.antenna.graph.RfDefaults;
import com.example.evanscomputermod.radio.antenna.solver.Ground;
import com.example.evanscomputermod.radio.api.AntennaPattern;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

import java.util.List;
import java.util.Map;

/**
 * The antenna tools' data shaping (what the {@code antenna} peripheral returns
 * and the analyzer screen plots) on a solved 7 MHz block dipole, plus the
 * pattern cuts on analytic patterns and the plot geometry.
 */
public class AntennaToolsDataTest {
    static AntennaReport dipole;

    /** A 2 x 10 block copper dipole 10 m over average ground (the ham_dipole scenario). */
    @BeforeAll
    static void solve() {
        double y = 10.5;
        var b = AntennaGraph.builder(new AntennaGraph.Point(0, y, 0.5), new AntennaGraph.Point(1, y, 0.5));
        AntennaGraph.blockRun(b, new AntennaGraph.Point(0, y, 0.5), -1, 0, 0, 10, RfDefaults.COPPER_WIRE);
        AntennaGraph.blockRun(b, new AntennaGraph.Point(1, y, 0.5), 1, 0, 0, 10, RfDefaults.COPPER_WIRE);
        b.ground(0, Ground.AVERAGE, "average soil");
        dipole = AntennaAnalysis.solve(b.build());
        assertEquals(AntennaReport.Status.SOLVED, dipole.status());
    }

    static double d(Object o) {
        return ((Number) o).doubleValue();
    }

    @Test
    void sweepFindsTheDipAtResonance() {
        List<Map<String, Object>> s = AntennaToolsData.sweep(dipole, 6e6, 8e6, 21);
        assertEquals(21, s.size());
        assertEquals(6e6, d(s.get(0).get("f")), 1e-6);
        assertEquals(8e6, d(s.get(20).get("f")), 1e-6);
        assertEquals(List.of("f", "swr", "r", "x"), List.copyOf(s.get(0).keySet()));
        int best = 0;
        for (int i = 0; i < s.size(); i++) if (d(s.get(i).get("swr")) < d(s.get(best).get("swr"))) best = i;
        double fBest = d(s.get(best).get("f"));
        assertEquals(dipole.resonantHz(), fBest, 0.1e6, "the sweep's best point is the resonance (0.1 MHz grid)");
        assertTrue(d(s.get(best).get("swr")) < 2, "matched at resonance");
        assertTrue(d(s.get(0).get("swr")) > 3 && d(s.get(20).get("swr")) > 3, "mismatched at the ends");
        // At resonance the reactance is small and the resistance near 50-75 ohms.
        Map<String, Object> z = AntennaToolsData.impedance(dipole, dipole.resonantHz());
        assertEquals(true, z.get("in_band"));
        assertTrue(Math.abs(d(z.get("x"))) < 15, "X at resonance " + z.get("x"));
        assertTrue(d(z.get("r")) > 35 && d(z.get("r")) < 110, "R at resonance " + z.get("r"));
        assertTrue(d(z.get("efficiency")) > 0.5);
    }

    @Test
    void outsideTheAnalysedBandsIsUnusableNotAnError() {
        Map<String, Object> z = AntennaToolsData.impedance(dipole, 5e9);
        assertEquals(false, z.get("in_band"));
        assertTrue(Double.isNaN(d(z.get("r"))));
        assertTrue(Double.isInfinite(d(z.get("swr"))));
        Map<String, Object> none = AntennaToolsData.impedance(AntennaReport.none("nothing"), 7e6);
        assertEquals(false, none.get("in_band"));
        assertEquals(0.0, d(none.get("efficiency")));
    }

    @Test
    void sweepArgumentsAreChecked() {
        assertThrows(IllegalArgumentException.class, () -> AntennaToolsData.sweep(dipole, 8e6, 6e6, 21));
        assertThrows(IllegalArgumentException.class, () -> AntennaToolsData.sweep(dipole, -1, 6e6, 21));
        assertThrows(IllegalArgumentException.class, () -> AntennaToolsData.sweep(dipole, 6e6, 8e6, 1));
        assertThrows(IllegalArgumentException.class, () -> AntennaToolsData.sweep(dipole, 6e6, 8e6, AntennaToolsData.MAX_POINTS + 1));
    }

    @Test
    void analyzerSpanCoversResonanceInsideTheCachedSweep() {
        double[] span = AntennaToolsData.analyzerSpan(dipole);
        assertNotNull(span);
        assertTrue(span[0] < dipole.resonantHz() && span[1] > dipole.resonantHz());
        double[] hz = AntennaToolsData.frequencies(span[0], span[1], 61);
        double[] swr = AntennaToolsData.swrCurve(dipole, hz);
        for (double s : swr) assertTrue(Double.isFinite(s), "the local sweep covers the whole span");
        SwrPlot plot = new SwrPlot(hz, swr);
        assertEquals(dipole.resonantHz(), hz[plot.minIndex()], (span[1] - span[0]) / 60 + 1);
        assertNull(AntennaToolsData.analyzerSpan(AntennaReport.none("x")));
    }

    @Test
    void powerLimitNamesTheWeakestLinkAndJudgesTheTransmitter() {
        var tx = new AntennaToolsData.Transmitter("amplifier 1kw", 1000);
        Map<String, Object> m = AntennaToolsData.powerLimit(dipole, 1, 10, 0, "evanscomputermod:copper_wire", tx);
        assertEquals(dipole.powerLimitW(), d(m.get("watts")));
        assertEquals("wire_current", m.get("cause"));
        @SuppressWarnings("unchecked") Map<String, Object> w = (Map<String, Object>) m.get("weakest");
        assertEquals(1, w.get("x"));
        assertEquals("copper wire", w.get("part"));
        assertEquals("will overheat", m.get("verdict"));
        String text = (String) m.get("text");
        assertTrue(text.startsWith("Limited to " + AntennaReport.watts(dipole.powerLimitW()) + " by copper wire at (1, 10, 0)"), text);
        assertTrue(text.endsWith("; amplifier 1kw 1.0 kW -> will overheat"), text);
        assertEquals("ok", AntennaToolsData.verdict(dipole, 5));
        assertTrue(AntennaToolsData.limitText(dipole, 1, 10, 0, new AntennaToolsData.Transmitter("sdr standard", 5)).endsWith("-> within rating"));
        assertFalse(AntennaToolsData.powerLimit(dipole, 0, 0, 0, "b", null).containsKey("verdict"));
        assertTrue(AntennaToolsData.limitText(AntennaReport.none("nothing here"), 0, 0, 0, null).startsWith("No antenna"));
    }

    /** An x-axis (east-west) horizontal dipole in free space: gain ∝ sin² from the wire axis. */
    static final AntennaPattern EW_DIPOLE = new AntennaPattern() {
        @Override public double gainDbi(double lx, double ly, double lz) {
            double s2 = Math.max(1e-9, 1 - lx * lx);
            return 2.15 + 10 * Math.log10(s2);
        }
        @Override public double[] polarization(double lx, double ly, double lz) { return new double[] {1, 0, 0}; }
        @Override public double peakGainDbi() { return 2.15; }
    };

    @Test
    void azimuthCutOfAHorizontalDipoleIsAFigureEight() {
        List<Double> az = AntennaToolsData.patternCut(EW_DIPOLE, "azimuth", 5, 0);
        assertEquals(72, az.size());
        assertEquals(2.15, az.get(0), 1e-9, "north: broadside");
        assertEquals(2.15, az.get(36), 1e-9, "south: broadside");
        assertTrue(az.get(18) < -60, "east: off the end of the wire");
        assertTrue(az.get(54) < -60, "west");
        // the elevation cut through north is omnidirectional (the wire is perpendicular to that plane)
        List<Double> el = AntennaToolsData.patternCut(EW_DIPOLE, "el", 30, 0);
        assertEquals(12, el.size());
        for (double g : el) assertEquals(2.15, g, 1e-9);
        assertThrows(IllegalArgumentException.class, () -> AntennaToolsData.patternCut(EW_DIPOLE, "sideways", 5, 0));
        assertThrows(IllegalArgumentException.class, () -> AntennaToolsData.patternCut(EW_DIPOLE, "az", 0, 0));
    }

    @Test
    void peakAndPolarization() {
        Map<String, Object> pk = AntennaToolsData.peak(EW_DIPOLE);
        assertEquals(2.15, d(pk.get("gain_dbi")), 1e-9);
        assertEquals(0.0, d(pk.get("az")) % 180, 1e-9, "broadside is north or south");
        Map<String, Object> pol = AntennaToolsData.polarization(EW_DIPOLE);
        assertEquals("horizontal", pol.get("sense"));
        assertEquals(0, d(pol.get("tilt_deg")), 1e-9);
        assertEquals("vertical", AntennaToolsData.polarization(AntennaPattern.VERTICAL_DIPOLE).get("sense"));
        assertEquals("slant", AntennaToolsData.polarizationName(45));
        // directions: bearing 90 = east (+x), elevation 90 = up
        double[] east = AntennaToolsData.direction(90, 0);
        assertEquals(1, east[0], 1e-12);
        double[] north = AntennaToolsData.direction(0, 0);
        assertEquals(-1, north[2], 1e-12);
        assertEquals(1, AntennaToolsData.direction(0, 90)[1], 1e-12);
        assertEquals(1, AntennaToolsData.elevationDirection(0, 90)[1], 1e-12);
        assertEquals(1, AntennaToolsData.elevationDirection(0, 180)[2], 1e-12, "180 in the north plane = south horizon");
    }

    @Test
    void solvedDipolePatternIsBroadsideToTheWire() {
        AntennaPattern p = dipole.patternAt(dipole.resonantHz());
        // a quarter wave up, the peak is high (NVIS); at low angles the wire along x
        // favours broadside (north/south) over end-fire (east/west)
        assertTrue(d(AntennaToolsData.peak(p).get("el")) >= 45, "NVIS peak " + AntennaToolsData.peak(p));
        List<Double> az = AntennaToolsData.patternCut(p, "azimuth", 10, 15);
        assertTrue(az.get(0) > az.get(9) + 3, "north " + az.get(0) + " vs east " + az.get(9));
        assertTrue(az.get(18) > az.get(27) + 3, "south " + az.get(18) + " vs west " + az.get(27));
        assertEquals("horizontal", AntennaToolsData.polarization(p).get("sense"));
    }

    @Test
    void swrPlotGeometry() {
        assertEquals(0, SwrPlot.yFrac(1), 1e-12);
        assertEquals(1, SwrPlot.yFrac(10), 1e-12);
        assertEquals(1, SwrPlot.yFrac(Double.POSITIVE_INFINITY), 1e-12);
        assertEquals(0.5, SwrPlot.yFrac(Math.sqrt(10)), 1e-12);
        assertEquals(10 + 99, SwrPlot.row(1, 10, 100));
        assertEquals(10, SwrPlot.row(50, 10, 100));
        SwrPlot p = new SwrPlot(new double[] {6e6, 7e6, 8e6}, new double[] {Double.POSITIVE_INFINITY, 1.4, 3});
        assertEquals(1, p.minIndex());
        assertEquals(0.5, p.xFracOf(7e6), 1e-12);
        assertEquals(50, p.col(1, 0, 101));
        assertEquals(-1, new SwrPlot(new double[] {1, 2}, new double[] {Double.NaN, Double.POSITIVE_INFINITY}).minIndex());
        assertEquals(List.of(6.0e6, 6.5e6, 7.0e6, 7.5e6, 8.0e6), SwrPlot.ticks(6e6, 8e6, 6));
        List<Double> t = SwrPlot.ticks(6.37e6, 8.63e6, 6);
        assertTrue(t.size() >= 3 && t.size() <= 6 && t.get(0) >= 6.37e6 && t.get(t.size() - 1) <= 8.63e6, t.toString());
        assertEquals("7.15", SwrPlot.label(7.15e6));
        assertEquals("7", SwrPlot.label(7e6));
        assertEquals("MHz", SwrPlot.unit(7e6));
        assertEquals("600", SwrPlot.label(600e3));
        assertThrows(IllegalArgumentException.class, () -> new SwrPlot(new double[2], new double[3]));
    }
}
