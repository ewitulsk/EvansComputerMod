package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.card.CardContent;
import com.example.evanscomputermod.card.CardReaderBlock;
import com.example.evanscomputermod.card.CardReaderBlockEntity;
import com.example.evanscomputermod.card.SimIdentity;
import com.example.evanscomputermod.card.SimSecret;
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.ui.Compositor;
import com.example.evanscomputermod.radio.antenna.RadioAntennaContent;
import com.example.evanscomputermod.radio.cellular.CellAntennaBlock;
import com.example.evanscomputermod.radio.cellular.CellularContent;
import com.example.evanscomputermod.radio.cellular.SimCardItem;
import com.example.evanscomputermod.radio.sdr.RadioSdrContent;
import com.example.evanscomputermod.radio.sdr.SdrBlock;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.Blocks;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Cellular v2 phase 3, the network side (docs/gui/CELLULAR_V2.md, "As built: phase 3 network
 * side"), spawnable with {@code /ecm scenario spawn cell_site_core} and run by the
 * {@code ecm_cellular} GameTest {@code cell_site_core}.
 *
 * <p>A player builds a carrier network from primitives and sets it up by typing:
 * <ol>
 *   <li>A <b>core</b> computer cabled (bottom) to a <b>server</b> on the LAN 10.0.0.0/24 and
 *       (top) to the cell site's computer on 10.0.1.0/24, with a Card Reader/Writer against its
 *       east side. {@code cored init cell-red 100.65.2.0/24} writes the subscriber database,
 *       {@code cored site add site-a} makes the site's credential, {@code cored start &} runs it.</li>
 *   <li>A <b>cell site</b>: a computer, an Advanced SDR, coax up a two-block mast and an LTE Sector
 *       Antenna on top. Control first: the player mistypes the credential in
 *       {@code celld-site config set ...}; {@code celld-site start &} is refused by the core
 *       (site: {@code core refused this site: authentication failed}; core: {@code bad credential}).
 *       With the right credential and {@code celld-site restart} the site registers over the
 *       sealed backhaul ({@code link up}, {@code cored sites} lists it connected).</li>
 *   <li>A SIM: a blank card clicked into the reader, {@code cored sim new --label alice} writes it
 *       (the card's key is the one in the database), the player takes it out by hand. Control: with
 *       the reader empty {@code cored sim new} says there is no card.</li>
 *   <li>A <b>probe</b>: another computer with an Advanced SDR in framed LTE UE mode on channel 3
 *       hears the site's system-information beacons ({@code framed recv}: from {@code a0000001},
 *       carrying the carrier name, ~10 per second).</li>
 *   <li>The core's GUI: the Carrier Console (a system app once the database exists) shows site-a
 *       connected.</li>
 * </ol>
 * The phone side (an LTE modem running {@code celld}) is the other phase-3 lane.
 *
 * <pre>
 *   y=4                                      panel(11,4,0)>                      &lt;panel(20,4,0)
 *   y=2,3                 cable (4..9,2,0)   coax mast (11,2..3,0)                coax (20,2..3,0)
 *   y=1   server(0,1,0)  core(4,1,0) reader(5,1,0)  site(9,1,0) SDR(10,1,0) coax(11,1,0)  coax(20,1,0) SDR(21,1,0) probe(22,1,0)
 *   y=0   cable (0..4,0,0); floor x -1..23, z -3..3
 * </pre>
 */
public final class CellNetScenarios {
    static final String PROMPT = PlayerKit.PROMPT;
    static final BlockPos SERVER = new BlockPos(0, 1, 0), CORE = new BlockPos(4, 1, 0), SITE = new BlockPos(9, 1, 0), PROBE = new BlockPos(22, 1, 0);
    static final BlockPos READER = CORE.east();
    static final String CARRIER = "cell-red";

    private CellNetScenarios() {}

    /** The SDR, coax and mast of the station at {@code pc}, growing east ({@code d} = 1) or west (-1). */
    static List<BlockPos> station(BlockPos pc, int d) {
        BlockPos c = pc.east(2 * d);
        return List.of(pc.east(d), c, c.above(), c.above(2), c.above(3));
    }

    static BlockPos panel(BlockPos pc, int d) {
        return pc.east(2 * d).above(3);
    }

    /** An Advanced SDR beside the computer, coax out one block and up a two-block mast, an LTE Sector Antenna on top. */
    static void buildStation(ScenarioRun r, BlockPos pc, int d) {
        buildStation(r, pc, d, d > 0 ? Direction.EAST : Direction.WEST, CellularContent.LTE_ANTENNA.get());
    }

    /**
     * {@link #buildStation}: the station grows east ({@code d} = 1) or west (-1) of the computer; {@code antenna} (the
     * LTE Sector Antenna or the mmWave Array Antenna) is clicked onto the top of the mast pointing {@code aim}.
     */
    static void buildStation(ScenarioRun r, BlockPos pc, int d, Direction aim, net.minecraft.world.level.block.Block antenna) {
        ScenarioPlayer p = r.player();
        Direction toward = d > 0 ? Direction.EAST : Direction.WEST, back = toward.getOpposite();
        BlockPos c = pc.east(2 * d);
        p.place(RadioSdrContent.SDR_ADVANCED.get(), r.abs(pc.east(d)), Direction.SOUTH, s -> s.getValue(SdrBlock.FACING) == Direction.SOUTH);
        p.placeAgainst(RadioAntennaContent.COAX_CABLE.get(), r.abs(c), Direction.SOUTH, back, null);
        p.placeAgainst(RadioAntennaContent.COAX_CABLE.get(), r.abs(c.above()), Direction.SOUTH, Direction.DOWN, null);
        p.placeAgainst(RadioAntennaContent.COAX_CABLE.get(), r.abs(c.above(2)), Direction.SOUTH, Direction.DOWN, null);
        p.placeAgainst(antenna, r.abs(panel(pc, d)), aim, Direction.DOWN, s -> s.getValue(CellAntennaBlock.FACING) == aim);
    }

    // ------------------------------------------------------------ GUI helpers (on the core)

    static ComputerInstance computer(ScenarioRun r, String host) {
        var be = r.terminal(host);
        return be == null ? null : be.getComputer();
    }

    static Compositor comp(ScenarioRun r) {
        var c = computer(r, "core");
        return c == null || c.getUiManager() == null ? null : c.getUiManager().compositor();
    }

    static String clickOn(ScenarioRun r, String layer, String text) {
        var c = comp(r);
        if (c == null) return "no GUI";
        int[] at = layer.equals("popup") ? c.locatePopupText(text, false) : c.locateLayerText(layer, text, false);
        if (at == null) return "\"" + text + "\" not shown on " + layer + " yet";
        r.click("core", at[0], at[1]);
        return null;
    }

    /** Click the text {@code text} in the window {@code title} on the core, as the player would. */
    static String clickText(ScenarioRun r, String title, String text) {
        var c = comp(r);
        if (c == null) return "no GUI";
        int[] at = c.findText(title, text);
        if (at == null) return "\"" + text + "\" not shown in " + title + " yet";
        r.click("core", at[0], at[1]);
        return null;
    }

    static String consoleShows(ScenarioRun r, String... texts) {
        var c = comp(r);
        if (c == null) return "no GUI";
        for (var w : c.windows()) {
            if (!w.title().equals("Carrier Console") || !w.mapped()) continue;
            var shown = c.textsOf(w.id());
            for (String t : texts) if (shown.stream().noneMatch(s -> s.contains(t))) return "the Carrier Console shows " + shown;
            return null;
        }
        return "no Carrier Console window";
    }

    // ------------------------------------------------------------ the SIM

    static CardReaderBlockEntity reader(ScenarioRun r) {
        return r.level().getBlockEntity(r.abs(READER)) instanceof CardReaderBlockEntity be ? be : null;
    }

    static String last(String text, String re) {
        Matcher m = Pattern.compile(re, Pattern.MULTILINE).matcher(text);
        String l = null;
        while (m.find()) l = m.group(1);
        return l;
    }

    /** The key the core's database holds for {@code imsi} (read from the computer's /etc/cored.db). */
    static String dbKey(ScenarioRun r, String imsi) {
        try {
            Path db = computer(r, "core").storagePath().resolve("etc/cored.db");
            return last(Files.readString(db), "^sub " + imsi + " key=([0-9a-f]{32})");
        } catch (Exception e) {
            return null;
        }
    }

    /** The reader holds the SIM cored wrote: its IMSI, the carrier, and the database's key. */
    static String readerHoldsWritten(ScenarioRun r) {
        String imsi = last(r.screen("core"), "^cored: wrote SIM IMSI (\\d{15})");
        CardReaderBlockEntity be = reader(r);
        if (imsi == null || be == null || !be.hasCard()) return "no written card in the reader yet";
        SimIdentity id = SimCardItem.identity(be.card());
        SimSecret s = SimCardItem.secret(be.card());
        if (id == null || !id.imsiText().equals(imsi) || !id.carrier().equals(CARRIER)) return "the reader holds " + id;
        String key = dbKey(r, imsi);
        if (key == null) return "no subscriber " + imsi + " in /etc/cored.db";
        if (s == null || !s.known() || !s.keyHex().equals(key)) {
            r.fail("the SIM's key is not the one in the core's database");
            return "failed";
        }
        r.say("§7  the reader holds SIM " + imsi + " (" + CARRIER + "), its key is the database's");
        return null;
    }

    static void insertBlank(ScenarioRun r) {
        List<String> said = r.player().use(new ItemStack(CellularContent.SIM_CARD.get()), r.abs(READER), Direction.NORTH, false);
        CardReaderBlockEntity be = reader(r);
        if (be == null || !be.hasCard()) throw new IllegalStateException("the reader took no card: the game said " + said);
        r.say("§7  blank SIM clicked into the Card Reader/Writer");
    }

    /** Right-click the reader with an empty hand: the card goes into the inventory. */
    static void takeOut(ScenarioRun r) {
        var inv = r.player().entity().getInventory();
        int was = inv.selected;
        inv.selected = 8; // an empty hotbar slot in hand; the card lands in the first free slot
        r.player().useEmpty(r.abs(READER), Direction.NORTH, false);
        inv.selected = was;
    }

    static String playerHasSim(ScenarioRun r) {
        String imsi = last(r.screen("core"), "^cored: wrote SIM IMSI (\\d{15})");
        var inv = r.player().entity().getInventory();
        for (int i = 0; i < inv.getContainerSize(); i++) {
            SimIdentity id = SimCardItem.identity(inv.getItem(i));
            if (id != null && id.imsiText().equals(imsi)) {
                CardReaderBlockEntity be = reader(r);
                if (be != null && be.hasCard()) return "the reader still holds a card";
                r.say("§7  the player has SIM " + imsi + " (alice)");
                return null;
            }
        }
        return "the SIM is not in the inventory";
    }

    // ------------------------------------------------------------ the scenario

    public static Scenario cellSiteCore() {
        List<BlockPos> floor = new ArrayList<>(PlayerKit.box(-1, 0, -3, 23, 0, 3));
        for (int x = 0; x <= 4; x++) floor.remove(new BlockPos(x, 0, 0)); // the LAN cable runs in the floor
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(station(SITE, 1));
        foot.addAll(station(PROBE, -1));
        foot.add(READER);
        String[] secret = {null};
        var b = Scenario.builder("cell_site_core",
                        "a carrier core (cored) with a Card Reader/Writer and a server on its LAN, and a cell site (celld-site on a computer"
                                + " with an Advanced SDR and an LTE panel on a mast) behind it: the site registers over the sealed backhaul,"
                                + " a SIM is written, a framed SDR hears the beacons, the Carrier Console shows the site; control: a wrong"
                                + " site credential is refused")
                .asPlayer()
                .realTime()
                .host("server", SERVER, "-")
                .host("core", CORE, "-")
                .host("site", SITE, "-")
                .host("probe", PROBE, "-")
                .link("lan", "server", Direction.DOWN, "core", Direction.DOWN, Scenario.Path.from(new BlockPos(0, 0, 0)).go(Direction.EAST, 4))
                .link("backhaul", "core", Direction.UP, "site", Direction.UP, Scenario.Path.from(new BlockPos(4, 2, 0)).go(Direction.EAST, 5))
                .decor(PlayerKit.decor(foot, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()), r -> {
                    r.player().place(CardContent.CARD_READER.get(), r.abs(READER), Direction.NORTH, s -> s.getValue(CardReaderBlock.FACING) == Direction.NORTH);
                    buildStation(r, SITE, 1);
                    buildStation(r, PROBE, -1);
                }, null))
                .timeLimit(60_000);
        b.note("Built for you: server and core cabled at their bottoms (LAN 10.0.0.0/24), core and site cabled at their tops"
                + " (10.0.1.0/24), a Card Reader/Writer against the core's east side; beside the site and the probe an Advanced SDR,"
                + " coax up a two-block mast and an LTE Sector Antenna on top, the two panels facing each other.");
        b.send("server", "ifconfig eth0 10.0.0.10/24");
        b.send("site", "ifconfig eth1 10.0.1.21/24");
        b.send("core", "ifconfig eth0 10.0.0.1/24");
        b.expect("core", "eth0: inet 10\\.0\\.0\\.1/24", "the core has 10.0.0.1 on the LAN");
        b.send("core", "ifconfig eth1 10.0.1.1/24");
        b.expect("core", "eth1: inet 10\\.0\\.1\\.1/24", "and 10.0.1.1 towards the site");
        b.expect("site", "eth1: inet 10\\.0\\.1\\.21/24", "the site has 10.0.1.21");

        b.note("core: the subscriber database, a credential for site-a, the core itself");
        b.send("core", "cored init " + CARRIER + " 100.65.2.0/24");
        b.expectOrFail("core", "^cored: carrier cell-red pool 100\\.65\\.2\\.0/24 gateway 100\\.65\\.2\\.1 in /etc/cored\\.db",
                "cored writes /etc/cored.db", "^cored: (?!carrier)");
        b.send("core", "cored site add site-a");
        b.expectOrFail("core", "^cored: site site-a added, secret [0-9a-f]{32}", "a credential for site-a", "^cored: (?!site)");
        b.mutate(r -> {
            secret[0] = last(r.screen("core"), "^cored: site site-a added, secret ([0-9a-f]{32})");
            if (secret[0] == null) throw new IllegalStateException("no secret on the core's screen");
        }, "read the credential off the core's screen");
        b.send("core", "cored start &");
        b.expect("core", "cored: carrier cell-red pool 100\\.65\\.2\\.0/24, cell0 100\\.65\\.2\\.1/24; sites connect to port 7700", "cored is running");

        b.note("site: settings with a mistyped credential (control): the core refuses the site");
        b.sendFn("site", r -> "celld-site config set site_id site-a secret " + secret[0].substring(0, 31) + (secret[0].endsWith("0") ? "1" : "0")
                        + " carrier cell-red core 10.0.1.1 cell_id 1 channel 3 power 0",
                "celld-site config set site_id site-a secret <the credential, last digit mistyped> carrier cell-red core 10.0.1.1 cell_id 1 channel 3 power 0");
        b.expectOrFail("site", "^celld-site: saved site_id, secret, carrier, core, cell_id, channel, power", "the settings are saved", "^celld-site: (?!saved)");
        b.send("site", "celld-site start &");
        b.expectOrFail("site", "celld-site: site-a cell 1 \\(cell-red\\) on lte channel 3, core 10\\.0\\.1\\.1:7700", "celld-site runs the radio",
                "^celld-site: (no framed radio|admin socket|not configured)");
        b.until("site", "celld-site status", "^link_error core refused this site: authentication failed", "control: the core refuses a wrong credential");
        b.until("core", "cored log", "site 'site-a' refused: bad credential", "the core logged the refusal");

        b.note("site: the right credential, restart: the site registers over the sealed backhaul");
        b.sendFn("site", r -> "celld-site config set secret " + secret[0], "celld-site config set secret <the credential>");
        b.expect("site", "^celld-site: saved secret", "saved");
        b.send("site", "celld-site restart");
        b.expect("site", "^celld-site: restarting", "the site restarts with its settings");
        b.until("site", "celld-site status", "^link up", "the site is registered with the core");
        b.send("core", "cored sites");
        b.expectOrFail("core", "^site site-a 1 10\\.0\\.1\\.21 1 \\d+ \\d+ 0", "cored lists site-a connected from 10.0.1.21 (cell 1)", "^site site-a 0 ");

        b.note("core: a SIM. Control: no card in the reader");
        b.send("core", "cored sim new --label alice");
        b.expectOrFail("core", "^cored: no card in the Card Reader/Writer", "control: no card, no SIM", "^cored: wrote");
        b.mutate(CellNetScenarios::insertBlank, "right-click the Card Reader/Writer with a blank SIM card");
        b.send("core", "cored sim new --label alice");
        b.expectOrFail("core", "^cored: wrote SIM IMSI 00101\\d{10} \\(alice\\)", "cored writes the SIM", "^cored: (?!wrote)");
        b.expect("core", "^cored: the card is released: take it out of the reader", "the card waits in the reader");
        b.await(CellNetScenarios::readerHoldsWritten, "the SIM holds the key the core's database holds", 3_000);
        b.mutate(CellNetScenarios::takeOut, "right-click the reader with an empty hand: take the SIM out");
        b.await(CellNetScenarios::playerHasSim, "the SIM is in the player's inventory", 2_000);

        b.note("probe: an Advanced SDR in framed LTE UE mode on channel 3 hears the site's beacons");
        b.send("probe", "framed config --phy lte --role ue --channel 3 --power 0 --addr 2");
        b.expect("probe", "^framed: lte ue channel 3", "the probe is an LTE UE on channel 3");
        b.send("probe", "framed recv --count 1 --seconds 5");
        b.expectOrFail("probe", "^rx a0000001 -> bcast seq \\d+ rssi -?\\d+\\.\\d dBm .* beacon 01[0-9a-f]*63656c6c2d726564",
                "a system-information beacon from the site's radio (a0000001), carrier cell-red", "^framed: 0 frames");
        b.send("probe", "framed recv --seconds 2 --quiet");
        b.expectOrFail("probe", "^framed: \\d+ frames \\((1[5-9]|2[0-2]) beacons\\)", "about 10 beacons a second", "^framed: \\d+ frames \\(([0-9]|1[0-4]) beacons\\)");

        b.note("core: the GUI; the Carrier Console (installed with the database) shows the site");
        b.send("core", "gui");
        b.expect("core", "^gui: started", "the desktop starts", "(?i)unknown command|no GUI device");
        b.await(r -> comp(r) != null && comp(r).shellConnected() ? null : "waiting for the shell", "the desktop is up", 15_000);
        b.await(r -> clickOn(r, "taskbar", "Apps"), "click Apps on the taskbar", 8_000);
        b.await(r -> clickOn(r, "popup", "Carrier Console"), "click Carrier Console in the launcher", 5_000);
        b.await(r -> consoleShows(r, "alice"), "the Carrier Console lists subscriber alice", 15_000);
        b.await(r -> clickText(r, "Carrier Console", "Sites"), "click Sites", 5_000);
        b.await(r -> consoleShows(r, "site-a", "connected"), "the Carrier Console: site-a connected", 5_000);
        return b.build();
    }
}
//?}
