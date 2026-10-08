package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.NetworkCableBlock;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.CableNetworkManager;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.compat.aero.AeroCompat;
import com.example.evanscomputermod.radio.compat.sable.SableShips;
import com.example.evanscomputermod.radio.medium.LinkCache;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import com.example.evanscomputermod.radio.medium.WorldRadioMedium;
import com.example.evanscomputermod.radio.wifi.ap.AccessPointBlockEntity;
import com.example.evanscomputermod.radio.wifi.ap.AccessPointContent;
import com.example.evanscomputermod.radio.wifi.ap.ApRadioPlan;
import com.example.evanscomputermod.radio.wifi.ap.ApSettings;
import com.example.evanscomputermod.radio.wifi.ap.IpResponder;
import com.example.evanscomputermod.radio.wifi.ap.VirtualStation;
import com.example.evanscomputermod.radio.wifi.ap.VirtualStations;
import com.example.evanscomputermod.radio.wifi80211.MacAddress;
import com.example.evanscomputermod.radio.wifi80211.Security;
import com.example.evanscomputermod.radio.wifi80211.ap.ClientStatus;
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

/**
 * Release-gate scenario {@code airship_radio} (spec: Sable and Create Aeronautics
 * support, Gates): an airship carrying a computer cabled to a Wi-Fi Access Point
 * flies away from a ground station and back.
 *
 * <pre>
 *   ship (y 8..12 above the floor):  deck of planks x -2..2, z -2..3 at y 8, with the
 *     cable (0,8,0)..(0,8,2) in it; computer "pc" (0,9,0) on the cable's start, AP (0,9,2)
 *     on its end; metal frame posts at the corners (Create girders, else iron bars);
 *     a balloon of envelope blocks at y 11..12 and an envelope skirt on the west side
 *     (x -3, y 9..10) that the radio path crosses (Aeronautics envelopes; wool without it).
 *   ground station: a virtual Wi-Fi phone (10.0.7.20, no computer behind it) in a wooden
 *     shack (x -9..-7, y 8..10, z 1..3) on a stone-brick pillar, 8 blocks west of the AP.
 * </pre>
 *
 * Both radios transmit at 0 dBm on channel 1 (WPA2-PSK). The script: the phone joins;
 * the pc pings it; the ship is assembled (Create Simulated's assembly helper with
 * Aeronautics, else Sable's) and the pc pings from the air; the ship flies east to 14 m
 * and 40 m (pings hold; RSSI and the rate the phone would use fall), then to 500 m
 * (beacon loss: the phone drops off, pings die; the medium skips middle-path diffraction
 * for the airborne path), then back to 14 m (the phone re-associates by itself, pings
 * come back), and finally sets down where it was built (AP settings, passphrase and the
 * pc's cable segment unchanged; pings still work). Controls: the 500 m leg, and the
 * level/rate comparison against the medium's own prediction.
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
    /** How long after a move the medium has certainly re-traced (AP pose refresh is 10 ticks) before sampling. */
    static final int SETTLE_TICKS = 30, SAMPLES = 40;

    private AirshipScenarios() {}

    /** Per-run state (the definition is shared by the command and the GameTest). */
    static final class State {
        SableShips.Ship ship;
        Vec3 home;
        long movedAt;
        int shift;
        final List<Double> rssi = new ArrayList<>();
        double nearRssi = Double.NaN, midRssi = Double.NaN;
        int nearRate, midRate;
        String settingsBefore;
        String passBefore;
        String log = "";
    }

    static final Map<ScenarioRun, State> STATE = Collections.synchronizedMap(new WeakHashMap<>());

    static State st(ScenarioRun r) {
        return STATE.computeIfAbsent(r, k -> new State());
    }

    public static String phoneKey(ScenarioRun run) {
        return "airship_radio@" + run.origin().toShortString() + "/phone";
    }

    static VirtualStation phone(ScenarioRun run) {
        return VirtualStations.get(phoneKey(run));
    }

    public static Scenario airshipRadio() {
        var b = Scenario.builder("airship_radio",
                        "an airship with a computer and a Wi-Fi AP flies away from a ground phone: pings hold at 14 and 40 m while RSSI"
                                + " and rate fall, the link drops at 500 m and re-associates on return; the ship lands with its AP"
                                + " config and cable network intact")
                .timeLimit(60_000);
        b.host("pc", PC, PC_IP + "/24");
        b.decor(new Airship());
        b.note("Ship: pc (10.0.7.1) cabled to an AP (SSID " + SSID + ", WPA2 \"" + PASS + "\", ch " + CHANNEL + ", " + TX_DBM
                + " dBm) under an envelope balloon. Ground: phone " + PHONE_IP + " in a wooden shack 8 blocks west.");
        b.configureHosts();
        b.await(AirshipScenarios::phoneJoined, "the phone joins the AP over WPA2", 20_000);
        b.until("pc", "ping " + PHONE_IP + " -n 1", "^1 packets sent, 1 received", "pc reaches the phone through the AP (ship still built in place)");
        b.expect("pc", "/ >", "shell");
        b.note("Assemble the airship (Create Simulated with Aeronautics, else Sable) and hold it where it was built");
        b.mutate(AirshipScenarios::assemble, "airship assembled");
        b.await(AirshipScenarios::onShip, "pc and AP run on the ship; the phone re-associated with the moved AP", 20_000);
        b.until("pc", "ping " + PHONE_IP + " -n 1", "^1 packets sent, 1 received", "ping from the airship");
        b.expect("pc", "/ >", "shell");
        b.note("Fly east: " + (NEAR + 8) + " m from the phone");
        b.mutate(r -> fly(r, NEAR), "ship " + (NEAR + 8) + " m from the phone");
        b.await(r -> sample(r, "near"), "RSSI sampled at " + (NEAR + 8) + " m", 15_000);
        b.until("pc", "ping " + PHONE_IP + " -n 1", "^1 packets sent, 1 received", "ping holds at " + (NEAR + 8) + " m");
        b.expect("pc", "/ >", "shell");
        b.note("Fly on: " + (MID + 8) + " m");
        b.mutate(r -> fly(r, MID), "ship " + (MID + 8) + " m from the phone");
        b.await(r -> sample(r, "mid"), "RSSI sampled at " + (MID + 8) + " m", 15_000);
        b.mutate(AirshipScenarios::compareLevels, "RSSI and rate fell with distance, as the medium predicts");
        b.until("pc", "ping " + PHONE_IP + " -n 1", "^1 packets sent, 1 received", "ping holds at " + (MID + 8) + " m");
        b.expect("pc", "/ >", "shell");
        b.note("Control: fly out to " + (FAR + 8) + " m, beyond range");
        b.mutate(r -> fly(r, FAR), "ship " + (FAR + 8) + " m from the phone");
        b.await(AirshipScenarios::linkLost, "the phone loses the AP's beacons and drops the link", 15_000);
        b.mutate(AirshipScenarios::checkFar, "out of range: the medium's predicted level is below the phone's sensitivity");
        b.ping("pc", PHONE_IP, 1, 0, "no reply from the phone " + (FAR + 8) + " m away");
        b.expect("pc", "/ >", "shell");
        b.note("Return to " + (NEAR + 8) + " m");
        b.mutate(r -> fly(r, NEAR), "ship back at " + (NEAR + 8) + " m");
        b.await(AirshipScenarios::phoneJoined, "the phone re-associates by itself", 20_000);
        b.until("pc", "ping " + PHONE_IP + " -n 1", "^1 packets sent, 1 received", "pings are back");
        b.expect("pc", "/ >", "shell");
        b.note("Land: set the ship down where it was built (disassemble)");
        b.mutate(AirshipScenarios::land, "ship disassembled onto its build site");
        b.await(AirshipScenarios::landed, "AP and pc back in the world, same AP identity/config/passphrase, same cable segment", 20_000);
        b.until("pc", "ping " + PHONE_IP + " -n 1", "^1 packets sent, 1 received", "pings after landing");
        b.mutate(AirshipScenarios::finish, "chunks released, phone stopped");
        return b.build();
    }

    // ------------------------------------------------------------ steps

    static AccessPointBlockEntity ap(ScenarioRun r) {
        State s = st(r);
        BlockPos at = s.ship != null && !s.ship.removed() ? s.ship.plotPos(r.abs(AP)) : r.abs(AP);
        return r.level().getBlockEntity(at) instanceof AccessPointBlockEntity a ? a : null;
    }

    /** Null when the phone is CONNECTED and the AP lists it AUTHORIZED. */
    static String phoneJoined(ScenarioRun r) {
        VirtualStation p = phone(r);
        AccessPointBlockEntity a = ap(r);
        if (p == null) return "no phone";
        if (a == null || a.core() == null) return "AP not running";
        ClientStatus c = a.core().client(p.mac());
        if (!p.connected()) return "phone " + p.core().state() + (p.core().lastError() == null ? "" : " (" + p.core().lastError() + ")");
        if (c == null || c.state() != ClientStatus.State.AUTHORIZED) return "AP sees the phone as " + c;
        return null;
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
        return phoneJoined(r);
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
    }

    /** Collect {@link #SAMPLES} beacon RSSIs at the phone once the move has settled; null when done. */
    static String sample(ScenarioRun r, String which) {
        State s = st(r);
        long since = r.level().getGameTime() - s.movedAt;
        if (since < SETTLE_TICKS) return "settling (" + since + " ticks)";
        VirtualStation p = phone(r);
        AccessPointBlockEntity a = ap(r);
        if (p == null || a == null) return "phone or AP missing";
        if (!p.connected()) return "phone not connected: " + p.core().lastError();
        double v = p.link().peerRssi(a.bssid());
        if (!Double.isNaN(v)) s.rssi.add(v);
        if (s.rssi.size() < SAMPLES) return s.rssi.size() + "/" + SAMPLES + " samples";
        double mean = s.rssi.stream().mapToDouble(Double::doubleValue).average().orElse(Double.NaN);
        int rate = ApRadioPlan.dataRate(CHANNEL, mean).kbps();
        double predicted = predictedRxDbm(r);
        ClientStatus c = a.core() == null ? null : a.core().client(p.mac());
        if (which.equals("near")) { s.nearRssi = mean; s.nearRate = rate; }
        else { s.midRssi = mean; s.midRate = rate; }
        log(r, "%s (%d m): phone hears the AP at %.1f dBm mean of %d (medium predicts %.1f dBm), rate %d kb/s; AP hears the phone at %s dBm, %s kb/s",
                which, s.shift + 8, mean, s.rssi.size(), predicted, rate,
                c == null ? "?" : c.lastRssiDbm(), c == null ? "?" : c.lastRateKbps());
        var m = RadioMediumHooks.medium();
        LinkCache.Link l = m instanceof WorldRadioMedium w ? w.link(a.link(), p.link(), ApRadioPlan.channel(CHANNEL).centerHz()) : null;
        if (Math.abs(mean - predicted) > 8) r.fail(which + ": mean RSSI " + fmt(mean) + " dBm is not the medium's predicted " + fmt(predicted) + " dBm");
        log(r, "  %s link traced at tick %s (moved at %d, now %d): AP %s phone %s; %s", which, l == null ? "-" : l.computedTick(), s.movedAt,
                r.level().getGameTime(), a.link().pose(), p.link().pose(), l == null ? "untraced" : l + "");
        return null;
    }

    /**
     * AP transmit power + the medium's cached link budget AP to phone (antenna gains and polarization from
     * the link, path loss from {@code pathGainDb}; no fading), dBm.
     */
    static double predictedRxDbm(ScenarioRun r) {
        AccessPointBlockEntity a = ap(r);
        VirtualStation p = phone(r);
        var m = RadioMediumHooks.medium();
        if (a == null || p == null || a.link() == null || !(m instanceof WorldRadioMedium w)) return Double.NaN;
        double f = ApRadioPlan.channel(CHANNEL).centerHz();
        LinkCache.Link l = w.link(a.link(), p.link(), f);
        if (l == null) return Double.NaN;
        return a.link().txPowerDbm() + l.gainA() + l.gainB() - l.polDb() + w.pathGainDb(a.link(), p.link(), f);
    }

    static void compareLevels(ScenarioRun r) {
        State s = st(r);
        if (!(s.midRssi < s.nearRssi - 3)) r.fail("RSSI did not fall with distance: " + fmt(s.nearRssi) + " -> " + fmt(s.midRssi) + " dBm");
        else if (!(s.midRate < s.nearRate)) r.fail("rate did not fall with distance: " + s.nearRate + " -> " + s.midRate + " kb/s");
        else log(r, "RSSI %.1f -> %.1f dBm, rate %d -> %d kb/s", s.nearRssi, s.midRssi, s.nearRate, s.midRate);
    }

    static String linkLost(ScenarioRun r) {
        VirtualStation p = phone(r);
        if (p == null) return "no phone";
        if (p.connected()) return "still connected (" + fmt(p.link().peerRssi(ap(r).bssid())) + " dBm last heard)";
        log(r, "far: %s", p.core().lastError());
        return null;
    }

    static void checkFar(ScenarioRun r) {
        double predicted = predictedRxDbm(r);
        AccessPointBlockEntity a = ap(r);
        VirtualStation p = phone(r);
        var m = RadioMediumHooks.medium();
        LinkCache.Link l = m instanceof WorldRadioMedium w ? w.link(a.link(), p.link(), ApRadioPlan.channel(CHANNEL).centerHz()) : null;
        log(r, "far (%d m): medium predicts %.1f dBm (sensitivity %.0f); path %s", FAR + 8, predicted, p.link().sensitivityDbm(),
                l == null ? "untraced" : l.path());
        if (!(predicted < p.link().sensitivityDbm())) r.fail("at " + (FAR + 8) + " m the medium still predicts " + fmt(predicted) + " dBm");
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
        return phoneJoined(r);
    }

    static void finish(ScenarioRun r) {
        Airship.release(r);
        VirtualStations.stop(phoneKey(r));
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
        st(r).log += line + "\n";
        EvansComputerMod.LOGGER.info("[airship_radio] {}", line);
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
            List<BlockPos> d = new ArrayList<>();
            for (int x = -2; x <= 2; x++) for (int z = -2; z <= 3; z++) for (int y = 11; y <= 12; y++) d.add(new BlockPos(x, y, z));
            return d;
        }

        static List<BlockPos> skirt() {
            List<BlockPos> d = new ArrayList<>();
            for (int z = -2; z <= 3; z++) for (int y = 9; y <= 10; y++) d.add(new BlockPos(-3, y, z));
            return d;
        }

        static List<BlockPos> shack() {
            List<BlockPos> d = new ArrayList<>();
            for (int x = -9; x <= -7; x++) for (int y = 8; y <= 10; y++) for (int z = 1; z <= 3; z++)
                if (!(x == PHONE.getX() && y == PHONE.getY() && z == PHONE.getZ())) d.add(new BlockPos(x, y, z));
            return d;
        }

        static List<BlockPos> pillar() {
            List<BlockPos> d = new ArrayList<>();
            for (int y = 1; y <= 7; y++) d.add(new BlockPos(PHONE.getX(), y, PHONE.getZ()));
            return d;
        }

        static List<BlockPos> floor() {
            List<BlockPos> d = new ArrayList<>();
            for (int x = -10; x <= 3; x++) for (int z = -3; z <= 4; z++) d.add(new BlockPos(x, 0, z));
            return d;
        }

        @Override
        public List<BlockPos> footprint() {
            List<BlockPos> all = new ArrayList<>(shipBlocks());
            all.addAll(shack());
            all.addAll(pillar());
            all.addAll(floor());
            all.add(PHONE);
            return all;
        }

        @Override
        public void build(ScenarioRun run) {
            var level = run.level();
            for (BlockPos p : floor()) level.setBlock(run.abs(p), Blocks.SMOOTH_STONE.defaultBlockState(), 3);
            for (BlockPos p : pillar()) level.setBlock(run.abs(p), Blocks.STONE_BRICKS.defaultBlockState(), 3);
            for (BlockPos p : shack()) level.setBlock(run.abs(p), Blocks.OAK_PLANKS.defaultBlockState(), 3);
            for (BlockPos p : deck()) level.setBlock(run.abs(p), Blocks.SPRUCE_PLANKS.defaultBlockState(), 3);
            BlockState frame = AeroCompat.metalFrame();
            for (BlockPos p : posts()) level.setBlock(run.abs(p), frame, 3);
            BlockState fabric = AeroCompat.envelope("white");
            for (BlockPos p : balloon()) level.setBlock(run.abs(p), fabric, 3);
            for (BlockPos p : skirt()) level.setBlock(run.abs(p), AeroCompat.envelope("red"), 3);
            for (BlockPos p : CABLE) level.setBlock(run.abs(p), ModBlocks.NETWORK_CABLE.get().defaultBlockState(), 3);
            level.setBlock(run.abs(AP), AccessPointContent.ACCESS_POINT.get().defaultBlockState(), 3);
            for (BlockPos rel : CABLE) {   // setBlock skips getStateForPlacement: connect the arms
                BlockPos p = run.abs(rel);
                BlockState s = level.getBlockState(p);
                for (Direction d : Direction.values()) {
                    BlockPos q = p.relative(d);
                    s = s.setValue(NetworkCableBlock.getPropertyForDirection(d),
                            NetworkCableBlock.canConnectToFace(level.getBlockState(q), d.getOpposite(), level, q));
                }
                level.setBlock(p, s, 3);
            }
            if (!(level.getBlockEntity(run.abs(AP)) instanceof AccessPointBlockEntity ap))
                throw new IllegalStateException("no access point at " + run.abs(AP));
            String err = ap.applySettings(new ApSettings(SSID, false, Security.WPA2_PSK, CHANNEL, TX_DBM, false, null, null), PASS);
            if (err != null) throw new IllegalStateException("AP settings rejected: " + err);
            // The ship flies out to FAR blocks east: keep that strip loaded (Sable unloads ships in unloaded chunks).
            BlockPos o = run.abs(BlockPos.ZERO);
            SableShips.forceChunks(level, o.getX() - 16, o.getX() + FAR + 16, o.getZ(), true);
            BlockPos p = run.abs(PHONE);
            String dim = level.dimension().location().toString();
            MacAddress mac = new MacAddress(0x025A_0000_0000L | ((p.asLong() & 0xFFFF) << 8) | 0x20);
            VirtualStation phone = new VirtualStation(mac, Pose.at(dim, p.getX() + 0.5, p.getY() + 0.5, p.getZ() + 0.5),
                    CHANNEL, SSID, PASS, p.asLong()).withIp(IpResponder.ip(PHONE_IP));
            phone.link().setTxPowerDbm(TX_DBM);
            VirtualStations.start(phoneKey(run), phone);
            st(run).log = "";
        }

        /** Release the strip of forced chunks and any ship still flying. */
        static void release(ScenarioRun run) {
            State s = STATE.get(run);
            if (s != null && s.ship != null && !s.ship.removed()) {
                SableShips.release(s.ship);
                SableShips.discard(s.ship);
            }
            BlockPos o = run.abs(BlockPos.ZERO);
            SableShips.forceChunks(run.level(), o.getX() - 16, o.getX() + FAR + 16, o.getZ(), false);
        }

        @Override
        public void clear(ScenarioRun run) {
            VirtualStations.stop(phoneKey(run));
            if (SableCompat.isLoaded()) release(run);
            STATE.remove(run);
        }
    }
}
//?}
