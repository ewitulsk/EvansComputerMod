package com.example.evanscomputermod.radio.antenna.tools;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.IPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import com.example.evanscomputermod.radio.antenna.Antenna;
import com.example.evanscomputermod.radio.antenna.AntennaManager;
import com.example.evanscomputermod.radio.antenna.graph.AntennaReport;
import com.example.evanscomputermod.radio.api.RadioCapabilities;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.conductor.Feedline;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.level.Level;
import org.jetbrains.annotations.Nullable;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * A feed point as a peripheral (type {@code antenna}): a computer next to a
 * feed point reads the antenna on it. Every method reads the antenna cache
 * ({@link AntennaManager#get}) on the server thread and never solves; while
 * the Method-of-Moments solve runs the answers are the heuristic estimate and
 * {@link #status} says {@code pending}.
 */
public final class AntennaPeripheral extends AnnotatedPeripheral {
    public static final String TYPE = "antenna";

    private final Level level;
    private final BlockPos feed;

    public AntennaPeripheral(Level level, BlockPos feed) {
        this.level = level;
        this.feed = feed.immutable();
    }

    public BlockPos feed() {
        return feed;
    }

    @Override
    public String getType() {
        return TYPE;
    }

    @Override
    public boolean isSame(IPeripheral other) {
        return other instanceof AntennaPeripheral a && a.level == level && a.feed.equals(feed);
    }

    private Antenna antenna() {
        return AntennaManager.get(level, feed);
    }

    private static double freq(double hz) throws PeripheralException {
        if (!(hz > 0) || !Double.isFinite(hz)) throw new PeripheralException("frequency must be a positive number of Hz");
        return hz;
    }

    @PeripheralMethod(description = "One line: resonance, 2:1 SWR band, power ratings (\"No antenna: ...\" without one)")
    public String summary() {
        return antenna().summary();
    }

    @PeripheralMethod(description = "{present, solved, pending, kind, summary, details, resonant_hz, analysis_hz, band_low_hz, band_high_hz, efficiency, gain_dbi, wire_m, segments, ground, feed:{x,y,z}}")
    public Map<String, Object> status() {
        Antenna a = antenna();
        AntennaReport r = a.report();
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("present", a.present());
        m.put("solved", a.solved());
        m.put("pending", a.pending());
        m.put("kind", r.kind());
        m.put("summary", a.summary());
        m.put("details", a.details());
        m.put("truncated", a.graph() != null && a.graph().truncated);
        m.put("resonant_hz", a.resonantHz());
        m.put("analysis_hz", r.analysisHz());
        m.put("band_low_hz", r.swrBandLowHz());
        m.put("band_high_hz", r.swrBandHighHz());
        m.put("efficiency", r.efficiency());
        m.put("gain_dbi", r.peakGainDbi());
        m.put("wire_m", r.wireLengthM());
        m.put("segments", r.segments());
        m.put("ground", r.groundName());
        m.put("feed", Map.of("x", feed.getX(), "y", feed.getY(), "z", feed.getZ()));
        return m;
    }

    @PeripheralMethod(description = "Lowest series resonance, Hz (NaN if none found or no antenna)")
    public double resonant_hz() {
        return antenna().resonantHz();
    }

    @PeripheralMethod(description = "Feed impedance at f Hz: {f, r, x, swr, efficiency, in_band}")
    public Map<String, Object> impedance(double hz) throws PeripheralException {
        return AntennaToolsData.impedance(antenna().report(), freq(hz));
    }

    @PeripheralMethod(description = "SWR against 50 ohms at f Hz (inf outside the analysed bands)")
    public double swr(double hz) throws PeripheralException {
        return antenna().swrAt(freq(hz));
    }

    @PeripheralMethod(description = "Radiation efficiency at f Hz, 0..1")
    public double efficiency(double hz) throws PeripheralException {
        return antenna().efficiencyAt(freq(hz));
    }

    @PeripheralMethod(description = "SWR sweep: list of {f, swr, r, x} from f_start to f_stop Hz (points 2..401, default 51)")
    public List<Map<String, Object>> sweep(double f0, double f1, Integer points) throws PeripheralException {
        try {
            return AntennaToolsData.sweep(antenna().report(), f0, f1, points == null ? 51 : points);
        } catch (IllegalArgumentException e) {
            throw new PeripheralException(e.getMessage());
        }
    }

    @PeripheralMethod(description = "Strongest direction at f Hz: {gain_dbi, az, el} (az: compass degrees, el: above the horizon)")
    public Map<String, Object> peak(double hz) throws PeripheralException {
        return AntennaToolsData.peak(antenna().pattern(freq(hz)));
    }

    @PeripheralMethod(description = "Gain cut in dBi, one value per step from 0 deg: plane 'azimuth' (at elevation angle, default the peak's) or 'elevation' (through azimuth angle, default the peak's)")
    public List<Double> pattern(double hz, String plane, Double stepDeg, Double angleDeg) throws PeripheralException {
        var p = antenna().pattern(freq(hz));
        try {
            boolean az = AntennaToolsData.isAzimuth(plane);
            double angle;
            if (angleDeg != null) angle = angleDeg;
            else angle = ((Number) AntennaToolsData.peak(p).get(az ? "el" : "az")).doubleValue();
            return AntennaToolsData.patternCut(p, plane, stepDeg == null ? 5 : stepDeg, angle);
        } catch (IllegalArgumentException e) {
            throw new PeripheralException(e.getMessage());
        }
    }

    @PeripheralMethod(description = "Polarization towards the peak at f Hz: {az, el, x, y, z, tilt_deg, sense}")
    public Map<String, Object> polarization(double hz) throws PeripheralException {
        return AntennaToolsData.polarization(antenna().pattern(freq(hz)));
    }

    @PeripheralMethod(description = "Power limit: {watts, wire_watts, voltage_watts, cause, weakest:{x,y,z,block,part}, transmitter, verdict, text}; amp_watts overrides the transmitter found on the feedline")
    public Map<String, Object> power_limit(Double ampWatts) {
        Antenna a = antenna();
        return powerLimit(level, a, ampWatts == null ? transmitter(level, feed) : new AntennaToolsData.Transmitter("amp", ampWatts));
    }

    /** {@link AntennaToolsData#powerLimit} for a live antenna (weakest-link block named from the world). */
    public static Map<String, Object> powerLimit(Level level, Antenna a, @Nullable AntennaToolsData.Transmitter tx) {
        BlockPos w = a.weakestLinkPos();
        String block = BuiltInRegistries.BLOCK.getKey(level.getBlockState(w).getBlock()).toString();
        return AntennaToolsData.powerLimit(a.report(), w.getX(), w.getY(), w.getZ(), block, tx);
    }

    /**
     * The transmitter driving the antenna: the block the feedline from the
     * feed point plugs into, if it is a radio endpoint (SDR, amplifier, ...),
     * at its maximum power. Null when nothing transmits into the feedline.
     */
    @Nullable
    public static AntennaToolsData.Transmitter transmitter(Level level, BlockPos feed) {
        Feedline line = Feedline.trace(level, feed);
        BlockPos end = line.end();
        if (end == null) return null;
        RadioEndpoint ep = level.getCapability(RadioCapabilities.ENDPOINT, end, null);
        if (ep == null) return null;
        double w = Math.pow(10, (ep.maxTxPowerDbm() - 30) / 10);
        String name = BuiltInRegistries.BLOCK.getKey(level.getBlockState(end).getBlock()).getPath().replace('_', ' ');
        return new AntennaToolsData.Transmitter(name, w);
    }
}
//?}
