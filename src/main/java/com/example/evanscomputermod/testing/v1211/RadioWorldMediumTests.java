package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.medium.LinkCache;
import com.example.evanscomputermod.radio.medium.WorldMediumContent;
import com.example.evanscomputermod.radio.medium.WorldRadioMedium;
import com.example.evanscomputermod.radio.phys.Materials;
import com.example.evanscomputermod.sable.SableCompat;
import com.example.evanscomputermod.testing.scenario.RadioScenarios;
import net.minecraft.core.BlockPos;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

import java.util.ArrayList;
import java.util.List;
import java.util.Locale;

/**
 * The in-world medium (lane 3A/5B/5D) in a real level, namespace {@code ecm_radio}:
 * walls attenuate by their {@code rf_attenuation} value, an iron room is a Faraday
 * cage, water blocks 2.4 GHz, HF goes through walls that stop Wi-Fi, and a Sable
 * hull between two stations attenuates their link. Each test builds its blocks,
 * registers {@link RadioTests.TestEndpoint}s, waits for the link cache to trace
 * the pairs (a few ticks) and then measures; every case has an open or glass control.
 */
@GameTestHolder(RadioTests.NS)
@PrefixGameTestTemplate(false)
public final class RadioWorldMediumTests {

    static final String NS = RadioTests.NS;
    static final String STRUCTURE = RadioTests.STRUCTURE;
    static final Channel WIFI = Channel.wifi24(6);
    static final Channel HF = new Channel(7.1e6, 3e3);

    /** Endpoints and blocks of one test, plus a tiny step machine around the link cache. */
    static final class Rig {
        final GameTestHelper h;
        final String dim;
        final List<RadioTests.TestEndpoint> eps = new ArrayList<>();
        final List<RadioTests.TestEndpoint[]> pairs = new ArrayList<>();
        final List<Double> freqs = new ArrayList<>();
        String failure;
        int waited;

        Rig(GameTestHelper h) {
            this.h = h;
            this.dim = h.getLevel().dimension().location().toString();
            // A stone floor so the ground under every path is the same.
            for (int x = 0; x < 30; x++) for (int z = 0; z < 30; z++) h.setBlock(new BlockPos(x, 0, z), Blocks.STONE.defaultBlockState());
        }

        WorldRadioMedium medium() {
            return WorldMediumContent.medium();
        }

        RadioTests.TestEndpoint at(double x, double y, double z, Channel ch) {
            BlockPos o = h.absolutePos(BlockPos.ZERO);
            RadioTests.TestEndpoint e = new RadioTests.TestEndpoint(Pose.at(dim, o.getX() + x, o.getY() + y, o.getZ() + z), ch);
            eps.add(e);
            return e;
        }

        RadioTests.TestEndpoint[] pair(double x0, double x1, double y, double z, Channel ch) {
            RadioTests.TestEndpoint[] p = {at(x0, y, z, ch), at(x1, y, z, ch)};
            pairs.add(p);
            freqs.add(ch.centerHz());
            return p;
        }

        void fill(int x0, int y0, int z0, int x1, int y1, int z1, BlockState s) {
            for (int x = x0; x <= x1; x++) for (int y = y0; y <= y1; y++) for (int z = z0; z <= z1; z++)
                h.setBlock(new BlockPos(x, y, z), s);
        }

        void shell(int x0, int y0, int z0, int x1, int y1, int z1, BlockState s) {
            for (int x = x0; x <= x1; x++) for (int y = y0; y <= y1; y++) for (int z = z0; z <= z1; z++)
                if (x == x0 || x == x1 || y == y0 || y == y1 || z == z0 || z == z1) h.setBlock(new BlockPos(x, y, z), s);
        }

        /** Register everything (once) and report whether every pair has been traced. */
        boolean traced() {
            WorldRadioMedium m = medium();
            if (m == null) {
                failure = "the in-world radio medium is not the server's medium: " + com.example.evanscomputermod.radio.medium.RadioMediumHooks.medium();
                return false;
            }
            if (waited++ == 0) for (var e : eps) m.register(e);
            boolean all = true;
            for (int i = 0; i < pairs.size(); i++)
                all &= !Double.isNaN(m.pathGainDb(pairs.get(i)[0], pairs.get(i)[1], freqs.get(i)));
            if (!all && waited > 200) failure = "pairs not traced after 200 ticks";
            return all;
        }

        double gain(RadioTests.TestEndpoint[] p, Channel ch) {
            return medium().pathGainDb(p[0], p[1], ch.centerHz());
        }

        /** Send one frame along the pair; true if it arrived. */
        boolean send(RadioTests.TestEndpoint[] p, Channel ch, double dbm, String modulation, double rate) {
            WorldRadioMedium m = medium();
            int before = p[1].got.size();
            m.transmit(p[0], Emission.frame(ch, dbm, m.nowMicros(), 500, modulation, rate, new byte[] {7, 7, 7, 7}));
            for (int i = before; i < p[1].got.size(); i++) if (p[1].got.get(i).from().equals(p[0].id())) return true;
            return false;
        }

        double lastRssi(RadioTests.TestEndpoint[] p) {
            for (int i = p[1].got.size() - 1; i >= 0; i--) if (p[1].got.get(i).from().equals(p[0].id())) return p[1].got.get(i).rssiDbm();
            return Double.NaN;
        }

        void done() {
            WorldRadioMedium m = medium();
            if (m != null) for (var e : eps) m.unregister(e);
        }

        void log(String fmt, Object... args) {
            EvansComputerMod.LOGGER.info("[ecm_radio] " + String.format(Locale.ROOT, fmt, args));
        }

        /** Drive: wait for tracing, then run {@code check} once (it sets {@link #failure} on a problem). */
        void run(String name, Runnable check) {
            boolean[] checked = {false};
            TestDriver.drive(h, NS, name, () -> {
                if (checked[0]) return failure == null;
                if (!traced()) return false;
                try {
                    for (int i = 0; i < pairs.size(); i++) {
                        LinkCache.Link l = medium().link(pairs.get(i)[0], pairs.get(i)[1], freqs.get(i));
                        log("%s pair %d: %s", name, i, l == null ? "untraced" : l.path());
                        for (var e : pairs.get(i)) {
                            var w = WorldMediumContent.rfWorld(h.getLevel());
                            int bx = (int) Math.floor(e.pose().x()), by = (int) Math.floor(e.pose().y()), bz = (int) Math.floor(e.pose().z());
                            log("  endpoint %s block %d %d %d surfaceY %d, above: %s / %s, level height %d", e.pose(), bx, by, bz,
                                    w.surfaceY(bx, bz), h.getLevel().getBlockState(new BlockPos(bx, by + 1, bz)),
                                    h.getLevel().getBlockState(new BlockPos(bx, by + 2, bz)),
                                    h.getLevel().getHeight(net.minecraft.world.level.levelgen.Heightmap.Types.MOTION_BLOCKING, bx, bz));
                        }
                    }
                    check.run();
                } finally {
                    checked[0] = true;
                    done();
                }
                return failure == null;
            }, () -> failure);
        }
    }

    /** (a) A one-block stone wall between two 2.4 GHz radios lowers the level by the table value (≈ 66 dB). */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".wall")
    public static void stone_wall_attenuates_wifi_by_table_value(GameTestHelper h) {
        Rig r = new Rig(h);
        r.fill(8, 1, 8, 8, 4, 12, Blocks.STONE.defaultBlockState());
        var open = r.pair(2.5, 14.5, 1.5, 24.5, WIFI);
        var wall = r.pair(2.5, 14.5, 1.5, 10.5, WIFI);
        r.run("stone_wall_attenuates_wifi_by_table_value", () -> {
            double table = Materials.STONE.dbPerBlock(WIFI.centerHz());
            double delta = r.gain(open, WIFI) - r.gain(wall, WIFI);
            r.log("open %.1f dB, wall %.1f dB, delta %.2f dB, table %.2f dB", r.gain(open, WIFI), r.gain(wall, WIFI), delta, table);
            if (Math.abs(delta - table) > 1) { r.failure = "wall delta " + delta + " vs table " + table; return; }
            // Received levels (with fading: Rician in the open, Rayleigh behind the wall) follow;
            // 50 dBm so the walled frame still decodes in a deep fade.
            if (!r.send(open, WIFI, 50, "DSSS-1", 1e6) || !r.send(wall, WIFI, 50, "DSSS-1", 1e6)) {
                r.failure = "a 50 dBm frame did not arrive (open " + open[1].got.size() + ", wall " + wall[1].got.size() + ")";
                return;
            }
            double rssiDelta = r.lastRssi(open) - r.lastRssi(wall);
            r.log("RSSI open %.1f, wall %.1f dBm", r.lastRssi(open), r.lastRssi(wall));
            if (Math.abs(rssiDelta - table) > 25) r.failure = "RSSI delta " + rssiDelta + " (fading bound 25 dB) vs " + table;
            // 10 dBm: through the wall the frame is lost (about -115 dBm), in the open it arrives.
            else if (!r.send(open, WIFI, 10, "DSSS-1", 1e6)) r.failure = "control: open-air 10 dBm frame lost";
            else if (r.send(wall, WIFI, 10, "DSSS-1", 1e6)) r.failure = "10 dBm frame got through a stone wall";
            else {
                // The debug command shows the same breakdown.
                String out = command(h, String.format(Locale.ROOT, "ecm radio link %.2f %.2f %.2f %.2f %.2f %.2f 2437",
                        wall[0].pose().x(), wall[0].pose().y(), wall[0].pose().z(), wall[1].pose().x(), wall[1].pose().y(), wall[1].pose().z()));
                r.log("/ecm radio link: %s", out);
                if (!out.contains(String.format(Locale.ROOT, "walls %.1f", table))) r.failure = "/ecm radio link output: " + out;
            }
        });
    }

    /** Run a command as an operator and return what it printed. */
    static String command(GameTestHelper h, String cmd) {
        StringBuilder out = new StringBuilder();
        var server = h.getLevel().getServer();
        net.minecraft.commands.CommandSource sink = new net.minecraft.commands.CommandSource() {
            @Override public void sendSystemMessage(net.minecraft.network.chat.Component c) { out.append(c.getString()).append('\n'); }
            @Override public boolean acceptsSuccess() { return true; }
            @Override public boolean acceptsFailure() { return true; }
            @Override public boolean shouldInformAdmins() { return false; }
        };
        var stack = new net.minecraft.commands.CommandSourceStack(sink, net.minecraft.world.phys.Vec3.atCenterOf(h.absolutePos(BlockPos.ZERO)),
                net.minecraft.world.phys.Vec2.ZERO, h.getLevel(), 4, "radio-test", net.minecraft.network.chat.Component.literal("radio-test"),
                server, null);
        server.getCommands().performPrefixedCommand(stack, cmd);
        return out.toString();
    }

    /** (b) A radio inside a closed iron room hears nothing; inside a glass room it does (control). */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".faraday")
    public static void iron_room_is_a_faraday_cage(GameTestHelper h) {
        Rig r = new Rig(h);
        r.shell(12, 1, 2, 16, 5, 6, Blocks.IRON_BLOCK.defaultBlockState());
        r.shell(12, 1, 12, 16, 5, 16, Blocks.GLASS.defaultBlockState());
        var iron = r.pair(2.5, 14.5, 2.5, 4.5, WIFI);
        var glass = r.pair(2.5, 14.5, 2.5, 14.5, WIFI);
        r.run("iron_room_is_a_faraday_cage", () -> {
            r.log("iron room %.1f dB, glass room %.1f dB", r.gain(iron, WIFI), r.gain(glass, WIFI));
            boolean i = r.send(iron, WIFI, 20, "OFDM-6", 6e6), g = r.send(glass, WIFI, 20, "OFDM-6", 6e6);
            if (i) r.failure = "a frame reached the radio inside the iron room";
            else if (!g) r.failure = "control: the frame did not reach the radio inside the glass room";
            else if (r.gain(glass, WIFI) - r.gain(iron, WIFI) < 60) r.failure = "iron vs glass only " + (r.gain(glass, WIFI) - r.gain(iron, WIFI)) + " dB";
        });
    }

    /** (c) Water blocks 2.4 GHz: a glass tank of water kills the link, the same tank empty does not. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".water")
    public static void water_blocks_wifi(GameTestHelper h) {
        Rig r = new Rig(h);
        for (int z : new int[] {4, 10}) {
            r.shell(7, 0, z - 1, 9, 3, z + 1, Blocks.GLASS.defaultBlockState());
            r.fill(8, 1, z, 8, 2, z, Blocks.AIR.defaultBlockState());
        }
        r.fill(8, 1, 4, 8, 2, 4, Blocks.WATER.defaultBlockState());
        var water = r.pair(2.5, 14.5, 1.5, 4.5, WIFI);
        var empty = r.pair(2.5, 14.5, 1.5, 10.5, WIFI);
        r.run("water_blocks_wifi", () -> {
            double loss = r.gain(empty, WIFI) - r.gain(water, WIFI);
            r.log("water tank %.1f dB, empty tank %.1f dB (water costs %.1f dB)", r.gain(water, WIFI), r.gain(empty, WIFI), loss);
            if (loss < 200) r.failure = "one block of water only cost " + loss + " dB at 2.4 GHz";
            else if (r.send(water, WIFI, 20, "DSSS-1", 1e6)) r.failure = "frame went through water";
            else if (!r.send(empty, WIFI, 20, "DSSS-1", 1e6)) r.failure = "control: frame lost through the empty glass tank";
        });
    }

    /** (d) HF (7.1 MHz) passes a 3-block stone wall that stops 2.4 GHz. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".hf_wall")
    public static void hf_passes_wall_that_blocks_wifi(GameTestHelper h) {
        Rig r = new Rig(h);
        r.fill(7, 1, 2, 9, 5, 14, Blocks.STONE.defaultBlockState());
        var hf = r.pair(2.5, 14.5, 1.5, 5.5, HF);
        var wifi = r.pair(2.5, 14.5, 1.5, 10.5, WIFI);
        r.run("hf_passes_wall_that_blocks_wifi", () -> {
            LinkCache.Link l = r.medium().link(hf[0], hf[1], HF.centerHz());
            r.log("HF %.1f dB (walls %.2f dB), Wi-Fi %.1f dB", r.gain(hf, HF), l == null ? Double.NaN : l.path().obstructionDb(), r.gain(wifi, WIFI));
            if (!r.send(hf, HF, 20, "BPSK", 300)) r.failure = "HF frame did not pass the stone wall";
            else if (r.send(wifi, WIFI, 20, "DSSS-1", 1e6)) r.failure = "control: 2.4 GHz frame passed 3 blocks of stone";
        });
    }

    /**
     * (e) Sable: an iron box assembled into a sub-level between two ground stations
     * attenuates their link (the trace walks the sub-level's blocks in plot space;
     * the world blocks are gone after assembly); an open lane is the control.
     * Skips with a logged reason when Sable isn't in the run.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".sable_hull")
    public static void sable_hull_attenuates_link(GameTestHelper h) {
        if (!SableCompat.isLoaded()) {
            EvansComputerMod.LOGGER.warn("[ecm_radio] sable_hull_attenuates_link SKIPPED: Sable is not loaded in this run");
            TestDriver.drive(h, NS, "sable_hull_attenuates_link", () -> true, () -> null);
            return;
        }
        Rig r = new Rig(h);
        r.shell(7, 1, 8, 10, 4, 11, Blocks.IRON_BLOCK.defaultBlockState());
        var hull = r.pair(2.5, 16.5, 2.5, 9.5, WIFI);
        var open = r.pair(2.5, 16.5, 2.5, 3.5, WIFI);
        long[] assembledAt = {-1};
        boolean[] checked = {false};
        TestDriver.drive(h, NS, "sable_hull_attenuates_link", () -> {
            if (checked[0]) return r.failure == null;
            if (!r.traced()) return false;
            WorldRadioMedium m = r.medium();
            if (assembledAt[0] < 0) {
                r.log("before assembly (world blocks): hull lane %.1f dB, open %.1f dB", r.gain(hull, WIFI), r.gain(open, WIFI));
                List<BlockPos> blocks = new ArrayList<>();
                BlockPos min = h.absolutePos(new BlockPos(7, 1, 8)), max = h.absolutePos(new BlockPos(10, 4, 11));
                for (BlockPos p : BlockPos.betweenClosed(min, max))
                    if (!h.getLevel().getBlockState(p).isAir()) blocks.add(p.immutable());
                SableHull.assemble(h.getLevel(), min, max, blocks);
                assembledAt[0] = h.getLevel().getGameTime();
                if (!h.getLevel().getBlockState(min).isAir()) r.failure = "the hull did not leave the world on assembly";
                return false;
            }
            LinkCache.Link l = m.link(hull[0], hull[1], WIFI.centerHz());
            if (l == null || l.computedTick() <= assembledAt[0]) {
                if (h.getLevel().getGameTime() - assembledAt[0] > 200) r.failure = "hull lane not retraced after assembly";
                return false;
            }
            checked[0] = true;
            try {
                double delta = r.gain(open, WIFI) - r.gain(hull, WIFI);
                r.log("after assembly: hull lane %.1f dB (sub-level blocks %.1f dB), open %.1f dB", r.gain(hull, WIFI),
                        l.path().volumeDb(), r.gain(open, WIFI));
                if (l.path().volumeDb() < 150) r.failure = "the sub-level hull only added " + l.path().volumeDb() + " dB";
                else if (delta < 150) r.failure = "hull vs open lane only " + delta + " dB";
                else if (r.send(hull, WIFI, 20, "DSSS-1", 1e6)) r.failure = "frame passed the iron hull";
                else if (!r.send(open, WIFI, 20, "DSSS-1", 1e6)) r.failure = "control: open lane frame lost";
            } finally {
                r.done();
            }
            return r.failure == null;
        }, () -> r.failure);
    }

    /** Sable calls, isolated so the class loads without Sable. */
    private static final class SableHull {
        static void assemble(net.minecraft.server.level.ServerLevel level, BlockPos min, BlockPos max, List<BlockPos> blocks) {
            dev.ryanhcode.sable.api.SubLevelAssemblyHelper.assembleBlocks(level, min, blocks,
                    new dev.ryanhcode.sable.companion.math.BoundingBox3i(min, max));
        }
    }

    /** The wifi_walls scenario (one lane per wall material) as a test. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".wifi_walls")
    public static void wifi_walls(GameTestHelper h) {
        TestDriver.scenario(h, NS, RadioScenarios.ALL.get("wifi_walls"));
    }

    private RadioWorldMediumTests() {}
}
//?}
