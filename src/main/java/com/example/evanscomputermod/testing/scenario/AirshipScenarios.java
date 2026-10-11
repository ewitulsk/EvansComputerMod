package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.CableNetworkManager;
import com.example.evanscomputermod.radio.compat.aero.AeroCompat;
import com.example.evanscomputermod.radio.compat.sable.SableShips;
import com.example.evanscomputermod.radio.medium.LinkCache;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import com.example.evanscomputermod.radio.medium.WorldRadioMedium;
import com.example.evanscomputermod.radio.wifi.ap.AccessPointBlockEntity;
import com.example.evanscomputermod.radio.wifi.ap.AccessPointContent;
import com.example.evanscomputermod.radio.wifi.ap.ApRadioPlan;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.sable.SableCompat;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.Vec3;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.WeakHashMap;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Release-gate scenario {@code airship_radio} (spec: Sable and Create Aeronautics
 * support, Gates): an airship carrying a computer cabled to a Wi-Fi Access Point
 * flies away from a ground computer and back.
 *
 * <pre>
 *   ship (y 8..12 above the floor):  deck of planks x -2..2, z -2..3 at y 8, with the
 *     cable (0,8,0)..(0,8,2) in it; computer "pc" (0,9,0) on the cable's start, AP (0,9,2)
 *     on its end; metal frame posts at the corners (Create girders, else iron bars);
 *     a balloon of envelope blocks at y 11..12 and an envelope skirt on the west side
 *     (x -3, y 9..10) that the radio path crosses (Aeronautics envelopes; wool without it).
 *   ground station: computer "phone" (10.0.7.20 on wlan0, Wi-Fi module, wpa_supplicant) in a
 *     wooden shack (x -9..-7, y 8..10, z 1..3, window north) on a stone-brick pillar, 8 blocks west of the AP.
 * </pre>
 *
 * A player builds it (computers, cable, AP and module by right-clicks; the AP
 * set up in its screen as WPA2 on channel 1 at 0 dBm) and operates it from the
 * keyboards; the strip the ship flies over is kept loaded with /forceload.
 * The script: the phone joins; the pc pings it; the ship is assembled
 * (Create Simulated's assembly helper with Aeronautics, else Sable's) and the
 * pc pings from the air; the ship flies east to 14 m and 40 m (pings hold;
 * {@code iw dev wlan0 link} on the phone shows the signal falling, the rate
 * it implies falls, and it matches the medium's prediction), then to 500 m
 * (beacon loss: wpa_supplicant leaves COMPLETED, pings die), then back to 14 m
 * (the phone re-associates by itself, pings come back), and finally sets down
 * where it was built (AP settings, passphrase and the pc's cable segment
 * unchanged; pings still work).
 *
 * <p><b>Not player-equivalent:</b> assembling, flying and landing the ship are
 * scripted (Sable sub-level assembly and held positions), because a player
 * flies an airship with propellers, controls and the Physics Assembler's
 * held lever, which a scenario can't drive reliably. Everything radio is real.
 */
public final class AirshipScenarios {

    public static final String SSID = "ecm-airship", PASS = "up up and away";
    public static final String PC_IP = "10.0.7.1", PHONE_IP = "10.0.7.20";
    public static final int CHANNEL = 1, TX_DBM = 0;
    public static final BlockPos PC = new BlockPos(0, 9, 0), AP = new BlockPos(0, 9, 2);
    public static final BlockPos PHONE = new BlockPos(-8, 9, 2);
    public static final List<BlockPos> CABLE = List.of(new BlockPos(0, 8, 0), new BlockPos(0, 8, 1), new BlockPos(0, 8, 2));
    /** Ship offsets east of where it was built, in blocks. Phone-to-AP distance is 8 more. */
    public static final int NEAR = 6, MID = 32, FAR = 492;
    /** Ticks after a move before sampling (the AP's pose refresh is 10 ticks, then the link is re-traced). */
    static final int SETTLE_TICKS = 30, SAMPLES = 4;

    private AirshipScenarios() {}

    /** Per-run state (the definition is shared by the command and the GameTest). */
    static final class State {
        SableShips.Ship ship;
        Vec3 home;
        long movedAt;
        int shift;
        final List<Double> rssi = new ArrayList<>();
        boolean asked;
        double nearRssi = Double.NaN, midRssi = Double.NaN;
        int nearRate, midRate;
        String settingsBefore;
        String passBefore;
    }

    static final Map<ScenarioRun, State> STATE = Collections.synchronizedMap(new WeakHashMap<>());

    static State st(ScenarioRun r) {
        return STATE.computeIfAbsent(r, k -> new State());
    }

    public static Scenario airshipRadio() {
        var b = Scenario.builder("airship_radio",
                        "an airship with a computer and a Wi-Fi AP flies away from a ground computer: pings hold at 14 and 40 m"
                                + " while the signal and rate fall, the link drops at 500 m and re-associates on return; the ship lands"
                                + " with its AP config and cable network intact (flight is scripted)")
                .asPlayer()
                .realTime()
                .timeLimit(60_000);
        b.host("pc", PC, PC_IP + "/24");
        b.host("phone", PHONE, "-");
        b.decor(new Airship());
        b.note("Built for you: on the ship, pc (10.0.7.1) cabled to an AP set up in its screen (SSID " + SSID + ", WPA2 \"" + PASS
                + "\", ch " + CHANNEL + ", " + TX_DBM + " dBm) under an envelope balloon. On the ground, the phone computer"
                + " (Wi-Fi module) in a wooden shack 8 blocks west.");
        b.send("pc", "ifconfig eth0 " + PC_IP + "/24");
        b.expect("pc", "eth0: inet " + Pattern.quote(PC_IP), "pc has " + PC_IP);
        PlayerKit.joinWpa2(b, "phone", PHONE_IP + "/24", SSID, PASS);
        b.until("phone", "wpa_cli status", "^wpa_state=COMPLETED$", "the phone joined the AP over WPA2");
        b.until("pc", "ping " + PHONE_IP + " -n 1", "^1 packets sent, 1 received", "pc reaches the phone through the AP (ship still built in place)");
        b.expect("pc", "/ >", "shell");
        b.note("[scripted] Assemble the airship (Create Simulated with Aeronautics, else Sable) and hold it where it was built");
        b.mutate(AirshipScenarios::assemble, "airship assembled");
        b.await(AirshipScenarios::onShip, "pc and AP run on the ship, on one cable segment", 20_000);
        b.until("phone", "wpa_cli status", "^wpa_state=COMPLETED$", "the phone is (re)associated with the moved AP");
        b.until("pc", "ping " + PHONE_IP + " -n 1", "^1 packets sent, 1 received", "ping from the airship");
        b.expect("pc", "/ >", "shell");
        b.note("[scripted] Fly east: " + (NEAR + 8) + " m from the phone");
        b.mutate(r -> fly(r, NEAR), "ship " + (NEAR + 8) + " m from the phone");
        b.await(r -> sample(r, "near"), "phone: iw dev wlan0 link, " + SAMPLES + " readings at " + (NEAR + 8) + " m", 20_000);
        b.until("pc", "ping " + PHONE_IP + " -n 1", "^1 packets sent, 1 received", "ping holds at " + (NEAR + 8) + " m");
        b.expect("pc", "/ >", "shell");
        b.note("[scripted] Fly on: " + (MID + 8) + " m");
        b.mutate(r -> fly(r, MID), "ship " + (MID + 8) + " m from the phone");
        b.await(r -> sample(r, "mid"), "phone: iw dev wlan0 link, " + SAMPLES + " readings at " + (MID + 8) + " m", 20_000);
        b.mutate(AirshipScenarios::compareLevels, "signal and rate fell with distance, as the medium predicts");
        b.until("pc", "ping " + PHONE_IP + " -n 1", "^1 packets sent, 1 received", "ping holds at " + (MID + 8) + " m");
        b.expect("pc", "/ >", "shell");
        b.note("Control: [scripted] fly out to " + (FAR + 8) + " m, beyond range");
        b.mutate(r -> fly(r, FAR), "ship " + (FAR + 8) + " m from the phone");
        b.until("phone", "wpa_cli status", "^wpa_state=(?!COMPLETED)\\S+$", "the phone loses the AP's beacons and drops the link");
        b.mutate(AirshipScenarios::checkFar, "out of range: the medium's predicted level is below the phone's sensitivity");
        b.ping("pc", PHONE_IP, 1, 0, "no reply from the phone " + (FAR + 8) + " m away");
        b.expect("pc", "/ >", "shell");
        b.note("[scripted] Return to " + (NEAR + 8) + " m");
        b.mutate(r -> fly(r, NEAR), "ship back at " + (NEAR + 8) + " m");
        b.until("phone", "wpa_cli status", "^wpa_state=COMPLETED$", "the phone re-associates by itself");
        b.until("pc", "ping " + PHONE_IP + " -n 1", "^1 packets sent, 1 received", "pings are back");
        b.expect("pc", "/ >", "shell");
        b.note("[scripted] Land: set the ship down where it was built (disassemble)");
        b.mutate(AirshipScenarios::land, "ship disassembled onto its build site");
        b.await(AirshipScenarios::landed, "AP and pc back in the world, same AP identity/config/passphrase, same cable segment", 20_000);
        b.until("phone", "wpa_cli status", "^wpa_state=COMPLETED$", "the phone is associated after landing");
        b.until("pc", "ping " + PHONE_IP + " -n 1", "^1 packets sent, 1 received", "pings after landing");
        b.mutate(AirshipScenarios::finish, "/forceload remove: the strip is released");
        return b.build();
    }

    // ------------------------------------------------------------ steps

    static AccessPointBlockEntity ap(ScenarioRun r) {
        State s = st(r);
        BlockPos at = s.ship != null && !s.ship.removed() ? s.ship.plotPos(r.abs(AP)) : r.abs(AP);
        return r.level().getBlockEntity(at) instanceof AccessPointBlockEntity a ? a : null;
    }

    static RadioEndpoint phoneRadio(ScenarioRun r) {
        return PlayerKit.moduleOf(r, "phone").endpoint();
    }

    static void assemble(ScenarioRun r) {
        State s = st(r);
        if (!SableCompat.isLoaded()) {
            r.fail("Sable is not loaded: airship_radio needs Sable (and Create Aeronautics for real envelopes)");
            return;
        }
        s.settingsBefore = String.valueOf(ap(r).settings());
        s.passBefore = passphrase(r, ap(r));
        List<BlockPos> blocks = new ArrayList<>();
        for (BlockPos rel : Airship.shipBlocks()) blocks.add(r.abs(rel));
        BlockPos min = r.abs(new BlockPos(-3, 8, -2)), max = r.abs(new BlockPos(2, 12, 3));
        s.ship = AeroCompat.assemble(r.level(), r.abs(AP), blocks, min, max);
        if (s.ship == null) {
            r.fail("assembly failed");
            return;
        }
        s.home = s.ship.position();
        SableShips.hold(s.ship, s.home.x, s.home.y, s.home.z, new double[] {0, 0, 0, 1});
        r.relocate("pc", s.ship.plotPos(r.abs(PC)));
        if (r.level().getBlockState(r.abs(AP)).is(AccessPointContent.ACCESS_POINT.get())) r.fail("the AP did not move onto the ship");
        log(r, "assembled by %s: %s, %d blocks", AeroCompat.lastAssembler, s.ship, blocks.size());
    }

    static String onShip(ScenarioRun r) {
        State s = st(r);
        TerminalBlockEntity pc = r.terminal("pc");
        if (pc == null || pc.getComputer() == null) return "no running pc at " + r.where("pc") + " (block there: "
                + r.level().getBlockState(r.where("pc")) + ", ship removed " + s.ship.removed() + ", at " + s.ship.position() + ")";
        AccessPointBlockEntity a = ap(r);
        if (a == null) return "no AP in the plot at " + s.ship.plotPos(r.abs(AP));
        if (!a.cabled()) return "AP not on a cable segment";
        if (!sameSegment(a, pc)) return "AP and pc not on one cable segment";
        return null;
    }

    static boolean sameSegment(AccessPointBlockEntity a, TerminalBlockEntity pc) {
        CableNetworkManager m = CableNetworkManager.getInstance();
        if (m == null || pc.getComputer() == null) return false;
        Integer net = m.networkOf(a.portMac());
        if (net == null) return false;
        for (byte[] mac : pc.getComputer().getNetworkMacs()) if (net.equals(m.networkOf(mac))) return true;
        return false;
    }

    static void fly(ScenarioRun r, int shift) {
        State s = st(r);
        if (s.ship == null || s.ship.removed()) {
            r.fail("no ship to fly");
            return;
        }
        s.shift = shift;
        SableShips.hold(s.ship, s.home.x + shift, s.home.y, s.home.z, new double[] {0, 0, 0, 1});
        s.movedAt = r.level().getGameTime();
        s.rssi.clear();
        s.asked = false;
    }

    static final Pattern SIGNAL = Pattern.compile("^\\s*signal: (-?\\d+(?:\\.\\d+)?) dBm", Pattern.MULTILINE);
    static final Pattern BITRATE = Pattern.compile("^\\s*tx bitrate: (\\d+(?:\\.\\d+)?) MBit/s", Pattern.MULTILINE);

    /**
     * Once the move has settled, type {@code iw dev wlan0 link} on the phone
     * until {@link #SAMPLES} signal readings are in; null when done.
     */
    static String sample(ScenarioRun r, String which) {
        State s = st(r);
        long since = r.level().getGameTime() - s.movedAt;
        if (since < SETTLE_TICKS) return "settling (" + since + " ticks)";
        if (!s.asked) {
            r.typeLine("phone", "iw dev wlan0 link");
            s.asked = true;
            return s.rssi.size() + "/" + SAMPLES + " readings";
        }
        String out = r.latestOutput("phone");
        if (out == null || !Pattern.compile(PlayerKit.PROMPT, Pattern.MULTILINE).matcher(out).find())
            return s.rssi.size() + "/" + SAMPLES + " readings (waiting for iw)";
        s.asked = false;
        Matcher m = SIGNAL.matcher(out);
        if (!m.find()) return "iw dev wlan0 link: " + out.strip();
        s.rssi.add(Double.parseDouble(m.group(1)));
        Matcher br = BITRATE.matcher(out);
        String bitrate = br.find() ? br.group(1) : "?";
        if (s.rssi.size() < SAMPLES) return s.rssi.size() + "/" + SAMPLES + " readings";
        double mean = s.rssi.stream().mapToDouble(Double::doubleValue).average().orElse(Double.NaN);
        int rate = ApRadioPlan.dataRate(CHANNEL, mean).kbps();
        double predicted = predictedRxDbm(r);
        if (which.equals("near")) { s.nearRssi = mean; s.nearRate = rate; }
        else { s.midRssi = mean; s.midRate = rate; }
        log(r, "%s (%d m): iw dev wlan0 link on the phone: signal %.1f dBm mean of %d %s (medium predicts %.1f dBm), last tx bitrate %s MBit/s;"
                        + " that level supports %d kb/s",
                which, s.shift + 8, mean, s.rssi.size(), s.rssi, predicted, bitrate, rate);
        if (Math.abs(mean - predicted) > 8) r.fail(which + ": mean signal " + fmt(mean) + " dBm is not the medium's predicted " + fmt(predicted) + " dBm");
        return null;
    }

    /**
     * AP transmit power + the medium's cached link budget AP to phone (antenna gains and polarization from
     * the link, path loss from {@code pathGainDb}; no fading), dBm.
     */
    static double predictedRxDbm(ScenarioRun r) {
        AccessPointBlockEntity a = ap(r);
        var m = RadioMediumHooks.medium();
        if (a == null || a.link() == null || !(m instanceof WorldRadioMedium w)) return Double.NaN;
        double f = ApRadioPlan.channel(CHANNEL).centerHz();
        RadioEndpoint p = phoneRadio(r);
        LinkCache.Link l = w.link(a.link(), p, f);
        if (l == null) return Double.NaN;
        return a.link().txPowerDbm() + l.gainA() + l.gainB() - l.polDb() + w.pathGainDb(a.link(), p, f);
    }

    static void compareLevels(ScenarioRun r) {
        State s = st(r);
        if (!(s.midRssi < s.nearRssi - 3)) r.fail("signal did not fall with distance: " + fmt(s.nearRssi) + " -> " + fmt(s.midRssi) + " dBm");
        else if (!(s.midRate < s.nearRate)) r.fail("rate did not fall with distance: " + s.nearRate + " -> " + s.midRate + " kb/s");
        else log(r, "signal %.1f -> %.1f dBm, rate %d -> %d kb/s", s.nearRssi, s.midRssi, s.nearRate, s.midRate);
    }

    static void checkFar(ScenarioRun r) {
        double predicted = predictedRxDbm(r);
        double sens = phoneRadio(r).sensitivityDbm();
        log(r, "far (%d m): medium predicts %.1f dBm (phone sensitivity %.0f)", FAR + 8, predicted, sens);
        if (!(predicted < sens)) r.fail("at " + (FAR + 8) + " m the medium still predicts " + fmt(predicted) + " dBm");
    }

    static void land(ScenarioRun r) {
        State s = st(r);
        BlockPos goal = r.abs(AP);
        AeroCompat.disassemble(s.ship, goal);
        r.relocate("pc", null);
        log(r, "landed by %s", AeroCompat.lastDisassembler);
    }

    static String landed(ScenarioRun r) {
        State s = st(r);
        if (s.ship != null && !s.ship.removed()) return "ship still in the air (" + s.ship + ")";
        if (!(r.level().getBlockEntity(r.abs(AP)) instanceof AccessPointBlockEntity a)) return "no AP at its build site " + r.abs(AP);
        TerminalBlockEntity pc = r.terminal("pc");
        if (pc == null || pc.getComputer() == null) return "no running pc at " + r.abs(PC);
        if (!String.valueOf(a.settings()).equals(s.settingsBefore)) {
            r.fail("AP settings changed: " + s.settingsBefore + " -> " + a.settings());
            return "failed";
        }
        if (!passphrase(r, a).equals(s.passBefore)) {
            r.fail("AP passphrase not kept server-side across assemble/fly/land");
            return "failed";
        }
        if (!sameSegment(a, pc)) return "AP and pc not yet on one cable segment";
        return null;
    }

    static void finish(ScenarioRun r) {
        Airship.release(r);
    }

    /** The passphrase as the server saves it (never in the client sync tag). */
    static String passphrase(ScenarioRun r, AccessPointBlockEntity a) {
        return a.saveWithoutMetadata(r.level().registryAccess()).getString(AccessPointBlockEntity.PASSPHRASE_KEY);
    }

    static String fmt(double v) {
        return String.format(Locale.ROOT, "%.1f", v);
    }

    static void log(ScenarioRun r, String f, Object... a) {
        String line = String.format(Locale.ROOT, f, a);
        EvansComputerMod.LOGGER.info("[airship_radio] {}", line);
        r.say("§7  " + line);
    }

    // ------------------------------------------------------------ layout

    static final class Airship implements Scenario.Decor {

        /** Every block of the ship (relative), terminal and AP included. */
        static List<BlockPos> shipBlocks() {
            List<BlockPos> out = new ArrayList<>(deck());
            out.addAll(CABLE);
            out.add(PC);
            out.add(AP);
            out.addAll(posts());
            out.addAll(balloon());
            out.addAll(skirt());
            return out;
        }

        static List<BlockPos> deck() {
            List<BlockPos> d = new ArrayList<>();
            for (int x = -2; x <= 2; x++) for (int z = -2; z <= 3; z++) {
                BlockPos p = new BlockPos(x, 8, z);
                if (!CABLE.contains(p)) d.add(p);
            }
            return d;
        }

        static List<BlockPos> posts() {
            List<BlockPos> d = new ArrayList<>();
            for (int x : new int[] {-2, 2}) for (int z : new int[] {-2, 3}) for (int y = 9; y <= 10; y++) d.add(new BlockPos(x, y, z));
            return d;
        }

        static List<BlockPos> balloon() {
            return PlayerKit.box(-2, 11, -2, 2, 12, 3);
        }

        static List<BlockPos> skirt() {
            return PlayerKit.box(-3, 9, -2, -3, 10, 3);
        }

        /** The shack's west wall block beside the phone: put up after the module is clicked into that side. */
        static final BlockPos SHACK_LAST = PHONE.west();
        /** The window in front of the phone's screen. */
        static final BlockPos WINDOW = PHONE.north();

        static List<BlockPos> shack() {
            List<BlockPos> d = new ArrayList<>();
            for (BlockPos p : PlayerKit.box(-9, 8, 1, -7, 10, 3))
                if (!p.equals(PHONE) && !p.equals(SHACK_LAST) && !p.equals(WINDOW)) d.add(p);
            return d;
        }

        static List<BlockPos> pillar() {
            return PlayerKit.box(PHONE.getX(), 1, PHONE.getZ(), PHONE.getX(), 7, PHONE.getZ());
        }

        static List<BlockPos> floor() {
            return PlayerKit.box(-10, 0, -3, 3, 0, 4);
        }

        @Override
        public List<BlockPos> footprint() {
            List<BlockPos> all = new ArrayList<>(shipBlocks());
            all.addAll(shack());
            all.addAll(pillar());
            all.addAll(floor());
            all.add(PHONE);
            all.add(SHACK_LAST);
            all.add(WINDOW);
            return all;
        }

        @Override
        public void terrain(ScenarioRun run) {
            PlayerKit.fill(run, floor(), Blocks.SMOOTH_STONE.defaultBlockState());
            PlayerKit.fill(run, pillar(), Blocks.STONE_BRICKS.defaultBlockState());
            PlayerKit.fill(run, shack(), Blocks.OAK_PLANKS.defaultBlockState());
            PlayerKit.fill(run, deck(), Blocks.SPRUCE_PLANKS.defaultBlockState());
        }

        @Override
        public void build(ScenarioRun run) {
            var level = run.level();
            // The radio part, by hand: cable through the deck, the AP on its end (set up in its screen), the phone's module.
            for (BlockPos p : CABLE) run.player().place(ModBlocks.NETWORK_CABLE.get(), run.abs(p), Direction.UP);
            PlayerKit.accessPoint(run, AP, Direction.NORTH, SSID, PASS, CHANNEL, TX_DBM);
            PlayerKit.wifiModules(run, "phone");
            level.setBlock(run.abs(SHACK_LAST), Blocks.OAK_PLANKS.defaultBlockState(), 3);
            // The hull's other materials (frames, envelope fabric), like the deck.
            BlockState frame = AeroCompat.metalFrame();
            PlayerKit.fill(run, posts(), frame);
            PlayerKit.fill(run, balloon(), AeroCompat.envelope("white"));
            PlayerKit.fill(run, skirt(), AeroCompat.envelope("red"));
            // The ship flies out to FAR blocks east: keep that strip loaded (Sable unloads ships in unloaded chunks).
            BlockPos o = run.abs(BlockPos.ZERO);
            run.say("§7  " + run.player().command(String.format(Locale.ROOT, "/forceload add %d %d %d %d",
                    o.getX() - 16, o.getZ(), o.getX() + FAR + 16, o.getZ())));
        }

        /** Release the strip of forced chunks and any ship still flying. */
        static void release(ScenarioRun run) {
            State s = STATE.get(run);
            if (s != null && s.ship != null && !s.ship.removed()) {
                SableShips.release(s.ship);
                SableShips.discard(s.ship);
            }
            BlockPos o = run.abs(BlockPos.ZERO);
            run.player().command(String.format(Locale.ROOT, "/forceload remove %d %d %d %d",
                    o.getX() - 16, o.getZ(), o.getX() + FAR + 16, o.getZ()));
        }

        @Override
        public void clear(ScenarioRun run) {
            if (SableCompat.isLoaded()) release(run);
            STATE.remove(run);
        }
    }
}
//?}
