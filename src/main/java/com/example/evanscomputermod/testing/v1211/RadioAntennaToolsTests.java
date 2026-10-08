package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.peripheral.IPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralCapability;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.radio.antenna.Antenna;
import com.example.evanscomputermod.radio.antenna.AntennaManager;
import com.example.evanscomputermod.radio.antenna.RadioAntennaContent;
import com.example.evanscomputermod.radio.antenna.tools.AnalyzerPackets;
import com.example.evanscomputermod.radio.antenna.tools.AntennaPeripheral;
import com.example.evanscomputermod.radio.antenna.tools.SwrPlot;
import com.example.evanscomputermod.radio.conductor.ConductorBlock;
import com.example.evanscomputermod.radio.conductor.FeedPointBlock;
import com.example.evanscomputermod.testing.scenario.RadioScenarios;
import io.netty.buffer.Unpooled;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.minecraft.network.FriendlyByteBuf;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

import java.util.List;
import java.util.Map;

/**
 * Antenna tools (lane 4E), namespace {@code ecm_radio}: the feed point's
 * {@code antenna} peripheral read directly on a built dipole (capability
 * lookup, sweep, impedance, pattern, limits, the analyzer screen's packet),
 * the bare-feed-point control, and the {@code antenna_tools} scenario (the
 * {@code antenna} program on booted computers).
 */
@GameTestHolder(RadioTests.NS)
@PrefixGameTestTemplate(false)
public final class RadioAntennaToolsTests {
    private static final String NS = RadioTests.NS;
    /** {@link RadioAntennaTests#dipole} builds here (relative). */
    private static final BlockPos FEED = new BlockPos(20, 12, 20);
    private static final BlockPos BARE = new BlockPos(5, 2, 5);

    private RadioAntennaToolsTests() {}

    private static double d(Object o) {
        return ((Number) o).doubleValue();
    }

    /**
     * A 7 MHz dipole read through the {@code antenna} peripheral found by
     * capability on the feed point: summary, a 6-8 MHz sweep dipping under
     * 2:1 at resonance, a small reactance at resonance, a broadside azimuth
     * cut, the copper-wire limit line, and the analyzer packet surviving its
     * codec. Control: a bare feed point reads "No antenna" and an unusable
     * sweep.
     */
    @GameTest(template = RadioTests.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".antenna_peripheral")
    public static void antenna_peripheral_reads_dipole(GameTestHelper h) {
        String[] failure = {null};
        RadioAntennaTests.dipole(h, 10, 10);
        ConductorBlock.placeConnected(h.getLevel(), h.absolutePos(BARE), RadioAntennaContent.FEED_POINT.get().defaultBlockState()
                .setValue(FeedPointBlock.AXIS, Direction.Axis.Z));
        RadioAntennaTests.steps(h, "antenna_peripheral_reads_dipole", failure, List.of(
                () -> AntennaManager.get(h.getLevel(), h.absolutePos(FEED)).solved(),
                () -> {
                    try {
                        check(h, failure);
                    } catch (PeripheralException | RuntimeException e) {
                        RadioAntennaTests.check(false, failure, "peripheral call failed: " + e);
                    }
                    return true;
                }));
    }

    private static void check(GameTestHelper h, String[] failure) throws PeripheralException {
        BlockPos feed = h.absolutePos(FEED);
        IPeripheral cap = h.getLevel().getCapability(PeripheralCapability.PERIPHERAL, feed, Direction.NORTH);
        RadioAntennaTests.check(cap instanceof AntennaPeripheral, failure, "no antenna peripheral on the feed point: " + cap);
        if (!(cap instanceof AntennaPeripheral p)) return;
        RadioAntennaTests.check(p.isSame(new AntennaPeripheral(h.getLevel(), feed)) && "antenna".equals(p.getType())
                && p.getMethodNames().containsAll(List.of("summary", "resonant_hz", "impedance", "swr", "efficiency", "sweep",
                "pattern", "power_limit", "polarization", "status", "peak")), failure, "peripheral identity / methods " + p.getMethodNames());
        Antenna a = AntennaManager.get(h.getLevel(), feed);
        double f = p.resonant_hz();
        RadioAntennaTests.check(p.summary().startsWith("Resonant at ") && f == a.resonantHz(), failure, "summary " + p.summary());

        List<Map<String, Object>> sweep = p.sweep(6e6, 8e6, 21);
        int best = 0;
        for (int i = 0; i < sweep.size(); i++) if (d(sweep.get(i).get("swr")) < d(sweep.get(best).get("swr"))) best = i;
        double fBest = d(sweep.get(best).get("f")), sBest = d(sweep.get(best).get("swr"));
        RadioAntennaTests.check(sweep.size() == 21 && Math.abs(fBest - f) <= 0.1e6 && sBest < 2, failure,
                "sweep best " + sBest + " at " + fBest + ", resonance " + f);
        RadioAntennaTests.check(d(sweep.get(0).get("swr")) > 3, failure, "no mismatch at 6 MHz: " + sweep.get(0));

        Map<String, Object> z = p.impedance(f);
        RadioAntennaTests.check(Math.abs(d(z.get("x"))) < 15 && d(z.get("r")) > 30 && Boolean.TRUE.equals(z.get("in_band")), failure, "Z at resonance " + z);
        RadioAntennaTests.check(Math.abs(p.swr(f) - d(z.get("swr"))) < 1e-9 && p.efficiency(f) > 0.5, failure, "swr/efficiency " + p.swr(f) + " " + p.efficiency(f));

        List<Double> az = p.pattern(f, "azimuth", 10.0, 15.0);
        RadioAntennaTests.check(az.size() == 36 && az.get(0) > az.get(9) + 3, failure, "azimuth cut north " + az.get(0) + " east " + az.get(9));
        RadioAntennaTests.check(p.pattern(f, "elevation", null, null).size() == 72, failure, "elevation cut size");
        RadioAntennaTests.check("horizontal".equals(p.polarization(f).get("sense")), failure, "polarization " + p.polarization(f));
        try {
            p.pattern(f, "sideways", null, null);
            RadioAntennaTests.check(false, failure, "bad plane accepted");
        } catch (PeripheralException expected) {
            // the plane is checked
        }

        Map<String, Object> lim = p.power_limit(1000.0);
        String text = (String) lim.get("text");
        RadioAntennaTests.check(text.startsWith("Limited to ") && text.contains(" by copper wire at (") && "will overheat".equals(lim.get("verdict")),
                failure, "limit " + lim);
        RadioAntennaTests.check(!p.power_limit(null).containsKey("transmitter"), failure, "transmitter found on an empty feedline");

        AnalyzerPackets.Plot plot = AnalyzerPackets.build(h.getLevel(), a);
        FriendlyByteBuf buf = new FriendlyByteBuf(Unpooled.buffer());
        AnalyzerPackets.Plot.STREAM_CODEC.encode(buf, plot);
        AnalyzerPackets.Plot back = AnalyzerPackets.Plot.STREAM_CODEC.decode(buf);
        SwrPlot sp = new SwrPlot(back.hz(), back.swr());
        RadioAntennaTests.check(back.hz().length == AnalyzerPackets.POINTS && back.summary().equals(plot.summary())
                && Math.abs(back.hz()[sp.minIndex()] - f) < 0.02 * f && back.swr()[sp.minIndex()] < 2 && back.limits().startsWith("Limited to"),
                failure, "analyzer plot " + back.summary() + " min " + (sp.minIndex() < 0 ? "none" : back.hz()[sp.minIndex()]));

        // Control: the bare feed point.
        IPeripheral bare = h.getLevel().getCapability(PeripheralCapability.PERIPHERAL, h.absolutePos(BARE), Direction.WEST);
        RadioAntennaTests.check(bare instanceof AntennaPeripheral, failure, "bare feed point has no peripheral");
        if (bare instanceof AntennaPeripheral b) {
            RadioAntennaTests.check(b.summary().startsWith("No antenna") && Boolean.FALSE.equals(b.status().get("present")), failure,
                    "bare feed point: " + b.summary());
            RadioAntennaTests.check(Double.isInfinite(d(b.sweep(6e6, 8e6, 5).get(2).get("swr"))), failure, "bare feed point sweep is usable");
            RadioAntennaTests.check(AnalyzerPackets.build(h.getLevel(), AntennaManager.get(h.getLevel(), h.absolutePos(BARE))).hz().length == 0,
                    failure, "bare feed point plot has points");
        }
        RadioAntennaTests.check(!(h.getLevel().getCapability(PeripheralCapability.PERIPHERAL, feed.east(), Direction.NORTH) instanceof AntennaPeripheral),
                failure, "a wire block exposes the antenna peripheral");
    }

    /** The {@code antenna} program on computers next to a dipole's feed point and a bare one. */
    @GameTest(template = RadioTests.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".antenna_tools")
    public static void antenna_tools(GameTestHelper h) {
        TestDriver.scenario(h, NS, RadioScenarios.ALL.get("antenna_tools"));
    }
}
//?}
