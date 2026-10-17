package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.card.CardContent;
import com.example.evanscomputermod.card.CardReaderBlock;
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.peripheral.PeripheralHub;
import com.example.evanscomputermod.computer.ui.PhoneUi;
import com.example.evanscomputermod.phone.PhoneHost;
import com.example.evanscomputermod.phone.PhoneMenu;
import com.example.evanscomputermod.radio.cellular.CellularContent;
import com.example.evanscomputermod.radio.cellular.FramedModemModule;
import com.example.evanscomputermod.radio.cellular.SimCardItem;
import com.example.evanscomputermod.radio.framed.PhyProfile;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.phys.Vec3;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Cellular v2 phase 3, the phone side (docs/gui/CELLULAR_V2.md "As built: phase 3 phone side").
 *
 * <ul>
 *   <li>{@code cell_phone_celld} (GameTest {@code ecm_phone}): a player writes a SIM at a Card
 *       Reader/Writer with {@code cardtool sim}, takes it out, holds a new ePhone and fits an LTE Modem and
 *       the SIM in its slot screen. The phone starts {@code celld} by itself; in the phone's Terminal
 *       {@code cell} shows the SIM (read through the {@code sim} peripheral of the SIM tray) and celld
 *       searching (no network is built here); the status bar has cellular bars and no carrier; {@code
 *       cell scan} hears nothing. Control: the SIM taken out again stops celld, and {@code cell} says
 *       celld isn't running.</li>
 *   <li>{@code cell_phone_attach} (GameTest {@code ecm_cellular}): a carrier core computer runs
 *       {@code cored} and writes the SIM with its Card Reader/Writer, a cell site (computer + Advanced
 *       SDR + coax + LTE Sector Antenna) runs {@code celld-site}, and the phone attaches, gets its address
 *       from the core and pings a server on the core's LAN; {@code cored status} lists it.</li>
 *   <li>{@code cell_chat} (GameTest {@code ecm_cellular}): the same network with a package and chat server
 *       on the core's LAN; the phone installs Chat from the App Store over LTE, chats with the server, and
 *       gets a notification in the background through background refresh.</li>
 * </ul>
 */
public final class CellPhoneScenarios {
    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();
    static final String PROMPT = PlayerKit.PROMPT;
    static final String IMSI = "310150000000001", CARRIER = "cell-red", KEY = "00112233445566778899aabbccddeeff";
    /** The card writer: a computer with a Card Reader/Writer on its east side, both on stone pedestals. */
    static final BlockPos WRITER = new BlockPos(4, 2, -2), READER = new BlockPos(5, 2, -2);

    private CellPhoneScenarios() {}

    static PhoneUi ui(ScenarioRun r) {
        PhoneHost h = PhoneScenarios.host(r);
        ComputerInstance c = h == null ? null : h.instance();
        return c == null || c.getUiManager() == null ? null : c.getUiManager().phone();
    }

    /** Inventory slot holding a written SIM, or -1. */
    static int simSlot(ScenarioRun r) {
        Inventory inv = PhoneScenarios.holder(r).getInventory();
        for (int i = 0; i < 36; i++) if (SimCardItem.isIssued(inv.getItem(i))) return i;
        return -1;
    }

    static int modemSlot(ScenarioRun r) {
        Inventory inv = PhoneScenarios.holder(r).getInventory();
        for (int i = 0; i < 36; i++) if (inv.getItem(i).is(CellularContent.modem(PhyProfile.LTE))) return i;
        return -1;
    }

    /** The phone slot screen's index of inventory slot {@code inv} (parts 0-3, inventory 4-30, hotbar 31-39). */
    static int menuIndex(int inv) {
        return inv < 9 ? PhoneMenu.SLOTS + 27 + inv : PhoneMenu.SLOTS + inv - 9;
    }

    /** Move an inventory stack from slot {@code from} to the empty slot {@code to}, as a player shuffles items. */
    static void shuffle(ScenarioRun r, int from, int to) {
        Inventory inv = PhoneScenarios.holder(r).getInventory();
        inv.setItem(to, inv.getItem(from));
        inv.setItem(from, ItemStack.EMPTY);
    }

    /** The card writer, the player's blank SIM written with cardtool sim and taken out by hand. */
    static void writeSim(Scenario.Builder b) {
        b.note("At the card writer: a blank SIM clicked into the Card Reader/Writer, written with cardtool sim, taken out by hand")
                .mutate(r -> r.player().use(new ItemStack(CellularContent.SIM_CARD.get()), r.abs(READER), Direction.NORTH, false),
                        "right-click the Card Reader/Writer with a blank SIM card")
                .send("pc", "cardtool sim write " + IMSI + " " + CARRIER + " --key " + KEY)
                .expectOrFail("pc", "^card released: waiting to be taken or pulled out$", "the SIM is written (" + CARRIER + ", IMSI " + IMSI + ")",
                        "^cardtool:")
                .expect("pc", PROMPT, "cardtool finished")
                .mutate(r -> {
                    Inventory inv = PhoneScenarios.holder(r).getInventory();
                    inv.selected = 8;   // an empty hand: the card goes into the inventory
                    r.player().useEmpty(r.abs(READER), Direction.NORTH, false);
                    int s = simSlot(r);
                    if (s < 0) {
                        r.fail("taking the card out gave the player no written SIM: " + r.player().heard());
                        return;
                    }
                    shuffle(r, s, 20);
                    inv.setItem(21, new ItemStack(CellularContent.modem(PhyProfile.LTE)));
                    r.say("§7  the written SIM is in the player's inventory, next to an LTE Modem");
                }, "take the SIM out of the reader (empty hand); the player also has an LTE Modem");
    }

    /** Sneak-right-click the phone in hand (its slot screen) and shift-click the LTE Modem and the SIM into it. */
    static void fitModemAndSim(ScenarioRun r) {
        r.player().useHeld(true);
        if (!(PhoneScenarios.holder(r).containerMenu instanceof PhoneMenu menu)) {
            r.fail("sneak-right-clicking the phone opened no slot screen");
            return;
        }
        int m = modemSlot(r), s = simSlot(r);
        if (m < 0 || s < 0) {
            r.fail("no modem (" + m + ") or SIM (" + s + ") in the inventory");
            return;
        }
        r.player().shiftClick(menuIndex(m));
        r.player().shiftClick(menuIndex(s));
        boolean modemIn = menu.getSlot(PhoneMenu.MODULE_1).getItem().is(CellularContent.modem(PhyProfile.LTE))
                || menu.getSlot(PhoneMenu.MODULE_2).getItem().is(CellularContent.modem(PhyProfile.LTE));
        boolean simIn = SimCardItem.isIssued(menu.getSlot(PhoneMenu.SIM).getItem());
        r.player().closeScreen();
        if (!modemIn || !simIn) r.fail("slot screen: modem in " + modemIn + ", SIM in " + simIn);
        else r.say("§7  slot screen: LTE Modem in a module slot, the SIM in the SIM tray");
    }

    /** Null once celld runs on the phone (started by PhoneUi). */
    static String celldRunning(ScenarioRun r) {
        PhoneUi ui = ui(r);
        if (ui == null) return "no phone GUI";
        return ui.celldPid() > 0 && ui.events().stream().anyMatch(e -> e.startsWith("celld start")) ? null : "events " + ui.events();
    }

    /** The phone's attachments: the SIM tray as {@code sim}, the modem as a framed radio, no SIM slot of the modem's own. */
    static String peripheralsLookRight(ScenarioRun r) {
        PhoneHost h = PhoneScenarios.host(r);
        PeripheralHub hub = h == null ? null : h.getPeripheralHub();
        if (hub == null) return "no hub";
        List<String> names = hub.names();
        String modem = null;
        for (String n : names) if (hub.get(n) instanceof FramedModemModule) modem = n;
        if (modem == null) return "no framed modem among " + names;
        if (hub.get("sim") == null || !"sim".equals(hub.get("sim").getType())) return "no sim peripheral among " + names;
        if (names.contains(modem + "_sim")) return "the phone's modem brought its own SIM slot: " + names;
        r.say("§7  attachments: " + names + " (" + modem + " = framed_radio, sim = the SIM tray)");
        return null;
    }

    /** The phone's status bar source ({@code /dev/uictl phone}) has {@code want}. */
    static String statusHas(ScenarioRun r, String want) {
        PhoneHost h = PhoneScenarios.host(r);
        ComputerInstance c = h == null ? null : h.instance();
        String s = c == null ? "" : c.getUiManager().control("phone");
        return s.contains(want) ? null : "uictl phone:\n" + s;
    }

    static void awaitScreen(Scenario.Builder b, String re, String what) {
        java.util.regex.Pattern p = java.util.regex.Pattern.compile(re, java.util.regex.Pattern.MULTILINE);
        b.await(r -> p.matcher(PhoneScenarios.screen(r)).find() ? null : "phone screen:\n" + PhoneScenarios.screen(r), what, 10_000);
    }

    // ------------------------------------------------------------ cell_phone_celld

    static List<BlockPos> writerFloor() {
        return PlayerKit.box(-2, 0, -4, 7, 0, 3);
    }

    static List<BlockPos> pedestals() {
        return List.of(WRITER.below(), READER.below());
    }

    static void buildWriter(ScenarioRun r) {
        r.player().place(CardContent.CARD_READER.get(), r.abs(READER), Direction.NORTH, s -> s.getValue(CardReaderBlock.FACING) == Direction.NORTH);
        r.say("§7  a Card Reader/Writer placed against the writer computer's east side");
    }

    static Scenario phoneCelld() {
        List<BlockPos> floor = writerFloor();
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(pedestals());
        foot.add(READER);
        var b = Scenario.builder("cell_phone_celld",
                        "an ePhone with an LTE Modem and a SIM runs celld by itself: the SIM is read through the SIM tray, celld searches"
                                + " (no network here), cell shows it; taking the SIM out stops celld")
                .asPlayer()
                .realTime()
                .host("pc", WRITER, "-")
                .decor(PlayerKit.decor(foot, r -> {
                    PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState());
                    PlayerKit.fill(r, pedestals(), Blocks.STONE.defaultBlockState());
                }, CellPhoneScenarios::buildWriter, PhoneScenarios::cleanup))
                .note("Setup (done for you): a computer with a Card Reader/Writer on its east side. No cell site or core: this is the"
                        + " phone side alone (cell_phone_attach is the whole network).");
        writeSim(b);
        b.mutate(PhoneScenarios::givePhone, "the player takes a new ePhone and holds it")
                .await(PhoneScenarios::booted, "the phone boots into its shell", 40_000)
                .note("Sneak-right-click the phone: its slot screen. Shift-click the LTE Modem and the SIM into it")
                .mutate(CellPhoneScenarios::fitModemAndSim, "slot screen: LTE Modem and SIM in")
                .await(CellPhoneScenarios::celldRunning, "the phone starts celld by itself (a framed modem and a SIM are in)", 15_000)
                .await(CellPhoneScenarios::peripheralsLookRight, "the modem is a framed radio and the SIM tray is the sim peripheral", 5_000)
                .await(r -> statusHas(r, "\ncell 0\n"), "the status bar shows cellular bars (none: no cell)", 15_000)
                .await(r -> statusHas(r, "\ncarrier -\n"), "and no carrier", 2_000);
        PhoneScenarios.openTerminal(b);
        PhoneScenarios.phoneRun(b, "cell", "^\\s+SIM\\s+cell-red, IMSI", "^cell: celld is not running", "cell: the SIM's identity from the SIM tray", 15_000);
        awaitScreen(b, "^\\s+state\\s+searching", "cell: celld is searching for a cell-red cell");
        PhoneScenarios.phoneRun(b, "cell scan", "^no cells heard", "^cell: celld is not running", "cell scan: nothing heard (no network)", 15_000);
        b.note("Control: take the SIM out of the phone (slot screen, shift-click the SIM tray)");
        b.mutate(r -> {
            r.player().useHeld(true);
            if (!(PhoneScenarios.holder(r).containerMenu instanceof PhoneMenu)) {
                r.fail("no slot screen");
                return;
            }
            r.player().shiftClick(PhoneMenu.SIM);
            r.player().closeScreen();
        }, "slot screen: the SIM back into the inventory");
        b.await(r -> {
            PhoneUi ui = ui(r);
            return ui != null && ui.celldPid() < 0 && ui.events().stream().anyMatch(e -> e.startsWith("celld stop")) ? null
                    : "events " + (ui == null ? "-" : ui.events());
        }, "celld stopped with the SIM gone", 10_000);
        PhoneScenarios.phoneRun(b, "cell", "^cell: celld is not running", "^\\s+state\\s", "cell: celld is not running", 15_000);
        b.timeLimit(60_000);
        return b.build();
    }

    // ------------------------------------------------------------ cell_phone_attach

    static final BlockPos CORE = new BlockPos(0, 1, -6), SITE = FramedScenarios.A, SERVER = new BlockPos(4, 1, -6);
    static final BlockPos CORE_READER = CORE.east();
    static final String CORE_IP = "10.0.40.1", SITE_IP = "10.0.40.2", LAN_IP = "10.0.41.1", SERVER_IP = "10.0.41.10";
    static final String SITE_SECRET = "5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e";

    /**
     * While the player works at the computers and the card writer (every click puts the main hand to use), the phone
     * rides in the off hand, where it is held (awake: it boots meanwhile), and the main hand is the empty slot 9.
     */
    static void phoneToOffHand(ScenarioRun r) {
        PhoneScenarios.move(r, PhoneScenarios.holder(r), 40);
        PhoneScenarios.select(PhoneScenarios.holder(r), 8);
    }

    /** The phone back in the main hand (hotbar slot 1). */
    static void phoneToHand(ScenarioRun r) {
        Inventory inv = PhoneScenarios.holder(r).getInventory();
        if (!inv.getItem(0).isEmpty()) for (int i = 9; i < 36; i++) if (inv.getItem(i).isEmpty()) { shuffle(r, 0, i); break; }
        PhoneScenarios.move(r, PhoneScenarios.holder(r), 0);
        PhoneScenarios.select(PhoneScenarios.holder(r), 0);
    }

    /** One line in the core's router CLI, then its prompt. */
    static void routerLine(Scenario.Builder b, String line) {
        b.send("core", line);
        b.expect("core", "^router\\S*#", "router: " + line);
    }

    /**
     * {@code cell_phone_attach} (GameTest {@code ecm_cellular cell_phone_attach}): the whole network. A core computer
     * (its router CLI: addresses and {@code ip routing}; {@code cored init}, the site's credential, {@code cored start})
     * with a Card Reader/Writer, cabled to a cell site (computer + Advanced SDR + coax + LTE Sector Antenna running
     * {@code celld-site}) and to a server. {@code cored sim new} writes the SIM; the player fits it and an LTE Modem in a
     * new ePhone and walks 17 blocks east of the antenna; celld attaches, gets 100.65.2.x from the core, the phone pings
     * the server through the core, {@code cored status} lists the phone's session at site-a.
     */
    static Scenario phoneAttach() {
        List<BlockPos> floor = new ArrayList<>(PlayerKit.box(-2, 0, -8, 26, 0, 3));
        List<BlockPos> cable = Scenario.Path.from(new BlockPos(0, 0, -6)).go(Direction.SOUTH, 6).blocks();
        floor.removeAll(cable);
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(cable);
        foot.addAll(FramedScenarios.station(SITE));
        foot.add(CORE_READER);
        String imsi = "001\\d{12}";
        var b = Scenario.builder("cell_phone_attach",
                        "a carrier core (cored, a Card Reader/Writer, a server on its LAN) and a player-built cell site (celld-site); an"
                                + " ePhone with an LTE Modem and a SIM written by cored attaches through celld, gets the core's address and"
                                + " pings the server; cored lists the phone")
                .asPlayer()
                .realTime()
                .host("core", CORE, "-")
                .host("site", SITE, "-")
                .host("server", SERVER, "-")
                .link("backhaul", "core", Direction.DOWN, "site", Direction.DOWN, Scenario.Path.from(new BlockPos(0, 0, -6)).go(Direction.SOUTH, 6))
                .link("lan", "core", Direction.UP, "server", Direction.UP, Scenario.Path.from(new BlockPos(0, 2, -6)).go(Direction.EAST, 4))
                .decor(PlayerKit.decor(foot, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()), r -> {
                    FramedScenarios.build(r, SITE);
                    r.player().place(CardContent.CARD_READER.get(), r.abs(CORE_READER), Direction.NORTH,
                            s -> s.getValue(CardReaderBlock.FACING) == Direction.NORTH);
                }, PhoneScenarios::cleanup))
                .note("Setup (done for you): the core cabled (bottom) to the cell site's computer and (top) to a server; a Card"
                        + " Reader/Writer against the core's east side; beside the site an Advanced SDR, coax and an LTE Sector Antenna"
                        + " pointing east.")
                .mutate(PhoneScenarios::givePhone, "the player takes a new ePhone (it boots while the network is built)")
                .mutate(CellPhoneScenarios::phoneToOffHand, "the phone into the off hand, the main hand free for building");
        b.send("server", "ifconfig eth1 " + SERVER_IP + "/24");
        b.send("site", "ifconfig eth0 " + SITE_IP + "/24");
        b.expect("server", "eth1: inet 10\\.0\\.41\\.10/24", "server has " + SERVER_IP);
        b.send("server", "ip route add 100.65.2.0/24 via " + LAN_IP + " dev eth1");
        b.expect("server", "^Route 100\\.65\\.2\\.0/24 added", "the server reaches the phones through the core");
        b.expect("site", "eth0: inet 10\\.0\\.40\\.2/24", "site has " + SITE_IP);
        b.note("core: addresses and IP forwarding in its router CLI (the phones' traffic leaves through it)");
        b.send("core", "router");
        b.expect("core", "^router#", "the router CLI");
        for (String l : List.of("configure terminal", "interface eth0", "ip address " + CORE_IP + "/24", "exit", "interface eth1",
                "ip address " + LAN_IP + "/24", "exit", "ip routing", "end")) routerLine(b, l);
        b.send("core", "exit");
        b.expect("core", PROMPT, "back in the shell");
        b.note("core: the subscriber database, site-a's credential, the core");
        b.send("core", "cored init " + CARRIER + " 100.65.2.0/24");
        b.expectOrFail("core", "^cored: carrier cell-red pool 100\\.65\\.2\\.0/24", "cored writes /etc/cored.db", "^cored: (?!carrier)");
        b.send("core", "cored site-add site-a " + SITE_SECRET);
        b.expect("core", "site site-a added", "site-a's credential");
        b.send("core", "cored start &");
        b.expect("core", "cored: carrier cell-red pool 100\\.65\\.2\\.0/24, cell0 100\\.65\\.2\\.1/24", "cored runs");
        b.note("site: settings, start");
        b.send("site", "celld-site config set site_id site-a secret " + SITE_SECRET + " carrier " + CARRIER + " core " + CORE_IP + " cell_id 1 channel 1");
        b.expectOrFail("site", "^celld-site: saved", "the site's settings", "^celld-site: (?!saved)");
        b.send("site", "celld-site start &");
        b.expectOrFail("site", "celld-site: site-a cell 1 \\(cell-red\\) on lte channel 1", "celld-site runs the radio", "^celld-site: (no framed radio|admin socket)");
        b.until("site", "celld-site status", "^link up", "the site is registered with the core");
        b.note("core: a SIM. A blank card clicked into the Card Reader/Writer, cored sim new, the card taken out by hand");
        b.mutate(r -> r.player().use(new ItemStack(CellularContent.SIM_CARD.get()), r.abs(CORE_READER), Direction.NORTH, false),
                "right-click the Card Reader/Writer with a blank SIM card");
        b.send("core", "cored sim new --label phone");
        b.expectOrFail("core", "^cored: wrote SIM IMSI " + imsi + " \\(phone\\)", "cored writes the SIM and puts it in the database", "^cored: (?!wrote)");
        b.mutate(r -> {
            r.player().useEmpty(r.abs(CORE_READER), Direction.NORTH, false);
            int s = simSlot(r);
            if (s < 0) {
                r.fail("taking the card out gave the player no written SIM: " + r.player().heard());
                return;
            }
            PhoneScenarios.holder(r).getInventory().setItem(21, new ItemStack(CellularContent.modem(PhyProfile.LTE)));
            r.say("§7  the written SIM is in the player's inventory, next to an LTE Modem");
        }, "take the SIM out of the reader (empty hand); the player also has an LTE Modem");
        b.await(r -> {
                    String why = PhoneScenarios.booted(r);
                    if (why != null && why.startsWith("no phone host")) {
                        var inv = PhoneScenarios.holder(r).getInventory();
                        StringBuilder sb = new StringBuilder(why + "; selected " + inv.selected + ";");
                        for (int i = 0; i < 9; i++) sb.append(' ').append(i).append('=').append(inv.getItem(i).getItem());
                        sb.append("; tracked players ").append(com.example.evanscomputermod.phone.PhoneServer.players(r.level().getServer()).size());
                        return sb.toString();
                    }
                    return why;
                }, "the phone has booted", 40_000)
                .mutate(r -> PhoneScenarios.standAt(r.player(), r, new Vec3(20.5, 0, 0.5)), "walk 17 blocks east of the site's antenna")
                .mutate(CellPhoneScenarios::phoneToHand, "the phone back in the main hand")
                .mutate(CellPhoneScenarios::fitModemAndSim, "slot screen: LTE Modem and SIM in")
                .await(CellPhoneScenarios::celldRunning, "celld runs", 15_000);
        PhoneScenarios.openTerminal(b);
        b.await(r -> {
            String why = statusHas(r, "\ncarrier " + CARRIER + "\n");
            if (why == null) return null;
            try {
                PhoneHost h = PhoneScenarios.host(r);
                String log = java.nio.file.Files.readString(h.instance().storagePath().resolve("var/log/celld.log"));
                return why + "\ncelld.log (end):\n" + log.substring(Math.max(0, log.length() - 1500));
            } catch (Exception e) {
                return why + " (no celld log: " + e + ")";
            }
        }, "the status bar shows " + CARRIER + " (attached)", 20_000);
        PhoneScenarios.phoneRun(b, "cell", "^\\s+state\\s+connected", "^cell: celld is not running", "cell: celld is connected", 15_000);
        PhoneScenarios.phoneRun(b, "ping " + SERVER_IP + " -n 3", "^3 packets sent, [1-3] received", "^3 packets sent, 0 received",
                "the phone pings the server on the core's LAN over LTE", 30_000);
        b.mutate(r -> PhoneScenarios.select(PhoneScenarios.holder(r), 8), "an empty main hand to type at the core");
        b.send("core", "cored status");
        b.expectOrFail("core", "^session \\d+ " + imsi + " 100\\.65\\.2\\.\\d+ active site-a 1", "cored lists the phone's session at site-a, cell 1",
                "^sessions 0");
        b.timeLimit(60_000);
        return b.build();
    }

    // ------------------------------------------------------------ cell_chat

    static final String CHAT = "dev.ecm.chat", STORE = "dev.ecm.store";
    static final String CHAT_CORE_IP = "10.0.70.1", CHAT_SITE_IP = "10.0.70.2", CHAT_LAN_IP = "10.0.71.1", CHAT_SERVER_IP = "10.0.71.10";
    static final String CHAT_POOL = "100.65.14.0/24";
    static final String CHAT_HELLO = "hello over lte", CHAT_REPLY = "hi pat, welcome", CHAT_LATER = "still there, pat?";

    /** Tap the text {@code text} in the phone's window {@code title} (its centre), as the holder's finger would. */
    static String tapIn(ScenarioRun r, String title, String text) {
        var c = PhoneGuiScenarios.comp(r);
        if (c == null) return "no phone GUI";
        int[] at = c.findText(title, text);
        if (at == null) return "\"" + text + "\" not shown in " + title + " yet";
        PhoneGuiScenarios.tap(r, at);
        return null;
    }

    static String chatShows(ScenarioRun r, String text) {
        var c = PhoneGuiScenarios.comp(r);
        if (c == null) return "no phone GUI";
        for (var w : c.windows()) {
            if (!w.appId().equals(CHAT) || !w.mapped()) continue;
            var texts = c.textsOf(w.id());
            return texts.stream().anyMatch(t -> t.contains(text)) ? null : "Chat shows " + texts;
        }
        return "no Chat window";
    }

    /**
     * Type a line at the server's keyboard: the phone goes to the off hand while the player's main hand is at the
     * keyboard (it stays held and awake there), and back into the main hand after.
     */
    static void atServer(Scenario.Builder b, String line) {
        b.mutate(CellPhoneScenarios::phoneToOffHand, "the phone into the off hand");
        b.send("server", line);
        b.mutate(CellPhoneScenarios::phoneToHand, "the phone back in the main hand");
    }

    static void typeOnPhone(Scenario.Builder b, String text) {
        b.mutate(r -> PhoneGuiScenarios.type(r, text), "type \"" + text + "\" on the phone");
    }

    /**
     * {@code cell_chat} (GameTest {@code ecm_cellular cell_chat}): a cell network (core + site, as in cell_phone_attach)
     * and, on the core's LAN, a server that runs {@code chatd}, packages the Chat app itself ({@code chatapp --source},
     * {@code pkg build}) and serves it ({@code pkg serve}). An ePhone with an LTE Modem and a SIM cored wrote attaches; in
     * the phone's App Store the player adds the server's repository, installs Chat over LTE and opens it; Connect asks
     * for the network (Allow); messages go both ways with the server's {@code chat} command. Then Chat goes to the
     * background (home): an ePhone suspends it, so in the phone's Terminal the player allows Chat background refresh and
     * notifications ({@code pkg perm grant}); the server writes again and the next refresh wake posts the notification.
     */
    static Scenario cellChat() {
        List<BlockPos> floor = new ArrayList<>(PlayerKit.box(-2, 0, -8, 26, 0, 3));
        List<BlockPos> cable = Scenario.Path.from(new BlockPos(0, 0, -6)).go(Direction.SOUTH, 6).blocks();
        floor.removeAll(cable);
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(cable);
        foot.addAll(FramedScenarios.station(SITE));
        foot.add(CORE_READER);
        var b = Scenario.builder("cell_chat",
                        "a cell network and a package server on the core's LAN: an ePhone installs the Chat app from the App Store over LTE,"
                                + " chats with the server's chat command, and gets a notification in the background (background refresh)")
                .asPlayer()
                .realTime()
                .host("core", CORE, "-")
                .host("site", SITE, "-")
                .host("server", SERVER, "-")
                .link("backhaul", "core", Direction.DOWN, "site", Direction.DOWN, Scenario.Path.from(new BlockPos(0, 0, -6)).go(Direction.SOUTH, 6))
                .link("lan", "core", Direction.UP, "server", Direction.UP, Scenario.Path.from(new BlockPos(0, 2, -6)).go(Direction.EAST, 4))
                .decor(PlayerKit.decor(foot, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()), r -> {
                    FramedScenarios.build(r, SITE);
                    r.player().place(CardContent.CARD_READER.get(), r.abs(CORE_READER), Direction.NORTH,
                            s -> s.getValue(CardReaderBlock.FACING) == Direction.NORTH);
                }, PhoneScenarios::cleanup))
                .note("Built for you: the core cabled (bottom) to the cell site's computer and (top) to a server; a Card Reader/Writer"
                        + " against the core's east side; beside the site an Advanced SDR, coax and an LTE Sector Antenna pointing east.")
                .mutate(r -> {
                    PhoneScenarios.givePhone(r);
                    // The battery drains per server tick, and a GameTest server ticks unthrottled while the phone runs in real time.
                    com.example.evanscomputermod.phone.PhoneConfig.override("battery.enabled", false);
                }, "the player takes a new ePhone (it boots while the network is built; batteries off for the run)")
                .mutate(CellPhoneScenarios::phoneToOffHand, "the phone into the off hand, the main hand free for building");
        b.send("server", "ifconfig eth1 " + CHAT_SERVER_IP + "/24");
        b.send("site", "ifconfig eth0 " + CHAT_SITE_IP + "/24");
        b.expect("server", "eth1: inet 10\\.0\\.71\\.10/24", "server has " + CHAT_SERVER_IP);
        b.send("server", "ip route add " + CHAT_POOL + " via " + CHAT_LAN_IP + " dev eth1");
        b.expect("server", "^Route 100\\.65\\.14\\.0/24 added", "the server reaches the phones through the core");
        b.note("server: the chat server, the Chat package, a repository");
        b.send("server", "chatd &");
        b.expect("server", "chatd listening on port 7777", "chatd listening");
        b.send("server", "chatapp --source /src/chat");
        b.expect("server", "chatapp: wrote Chat \\(dev\\.ecm\\.chat\\) 1\\.0\\.0 to /src/chat", "Chat's package folder", "^chatapp: can't");
        b.expect("server", PROMPT, "shell");
        b.send("server", "pkg build /src/chat /srv/repo");
        b.expect("server", "^Built /srv/repo/dev\\.ecm\\.chat-1\\.0\\.0\\.ecmpkg \\(3 files", "pkg build packs it", "^pkg: ");
        b.expect("server", PROMPT, "shell");
        b.send("server", "pkg serve /srv/repo 8080 &");
        b.expect("server", "Serving 1 packages from /srv/repo on port 8080", "the repository is online", "^pkg: ");
        b.expect("site", "eth0: inet 10\\.0\\.70\\.2/24", "site has " + CHAT_SITE_IP);
        CellularScenarios.routerCli(b, "core", "configure terminal", "interface eth0", "ip address " + CHAT_CORE_IP + "/24", "exit",
                "interface eth1", "ip address " + CHAT_LAN_IP + "/24", "exit", "ip routing", "end");
        CellularScenarios.coreUp(b, "core", CARRIER, CHAT_POOL, "site-a");
        CellularScenarios.siteUp(b, "site", "site-a", CARRIER, CHAT_CORE_IP, 1, PhyProfile.LTE, 1, "");
        b.note("core: a SIM. A blank card clicked into the Card Reader/Writer, cored sim new, the card taken out by hand");
        b.mutate(r -> r.player().use(new ItemStack(CellularContent.SIM_CARD.get()), r.abs(CORE_READER), Direction.NORTH, false),
                "right-click the Card Reader/Writer with a blank SIM card");
        b.send("core", "cored sim new --label pat");
        b.expectOrFail("core", "^cored: wrote SIM IMSI 001\\d{12} \\(pat\\)", "cored writes the SIM", "^cored: (?!wrote)");
        b.mutate(r -> {
            r.player().useEmpty(r.abs(CORE_READER), Direction.NORTH, false);
            if (simSlot(r) < 0) {
                r.fail("taking the card out gave the player no written SIM: " + r.player().heard());
                return;
            }
            PhoneScenarios.holder(r).getInventory().setItem(21, new ItemStack(CellularContent.modem(PhyProfile.LTE)));
        }, "take the SIM out of the reader (empty hand); the player also has an LTE Modem");
        b.await(PhoneGuiScenarios::guiBooted, "the phone has booted into its shell", 40_000)
                .mutate(r -> PhoneScenarios.standAt(r.player(), r, new Vec3(20.5, 0, 0.5)), "walk 17 blocks east of the site's antenna")
                .mutate(CellPhoneScenarios::phoneToHand, "the phone back in the main hand")
                .mutate(CellPhoneScenarios::fitModemAndSim, "slot screen: LTE Modem and SIM in")
                .await(CellPhoneScenarios::celldRunning, "celld runs", 15_000)
                .await(r -> statusHas(r, "\ncarrier " + CARRIER + "\n"), "the status bar shows " + CARRIER + " (attached)", 20_000);

        b.note("The App Store (home grid): Repos, the server's address, Add; Chat, Install (over LTE), Open");
        PhoneGuiScenarios.openApp(b, "App Store", STORE);
        b.await(r -> PhoneGuiScenarios.comp(r).findText("App Store", "No apps yet. Add a repository under Repos.") != null ? null : "store not loaded",
                "control: no repository, no apps", 15_000);
        b.await(r -> tapIn(r, "App Store", "Repos"), "tap Repos", 5_000);
        b.await(r -> tapIn(r, "App Store", "http://10.0.30.1:8080/"), "tap the address field", 5_000);
        typeOnPhone(b, CHAT_SERVER_IP + ":8080");
        b.await(r -> tapIn(r, "App Store", "Add"), "tap Add", 5_000);
        b.await(r -> PhoneGuiScenarios.comp(r).findText("App Store", "Chat") != null ? null : "Chat not listed",
                "the repository is added and lists Chat (its index fetched over LTE)", 20_000);
        b.await(r -> tapIn(r, "App Store", "Install"), "tap Install", 5_000);
        b.expect("server", "GET /dev\\.ecm\\.chat-1\\.0\\.0\\.ecmpkg", "the repository served the package");
        b.await(r -> PhoneGuiScenarios.computer(r).getAppManager().manifest(CHAT) != null ? null : "not installed yet",
                "Chat is installed on the phone", 30_000);
        b.await(r -> tapIn(r, "App Store", "Open"), "tap Open", 15_000);
        b.await(r -> PhoneGuiScenarios.onScreen(r, CHAT) != null ? null : "windows " + PhoneGuiScenarios.comp(r).windows(),
                "Chat opens full screen", 20_000);

        b.note("Chat: the server's address and a nickname, Connect; the phone asks for the network: Allow");
        b.await(r -> tapIn(r, "Chat", "10.0.30.1"), "tap the server field", 5_000);
        typeOnPhone(b, CHAT_SERVER_IP);
        b.await(r -> tapIn(r, "Chat", "Nickname (optional)"), "tap the nickname field", 5_000);
        typeOnPhone(b, "pat");
        b.await(r -> tapIn(r, "Chat", "Connect"), "tap Connect", 5_000);
        b.await(r -> String.join(" ", PhoneGuiScenarios.comp(r).layerTexts("overlay")).contains("network") ? null
                : "overlay " + PhoneGuiScenarios.comp(r).layerTexts("overlay"), "the phone asks to allow Chat the network", 20_000);
        PhoneGuiScenarios.tapShell(b, "overlay", "Allow", "tap Allow");
        b.await(r -> chatShows(r, "as pat"), "Chat is connected as pat (over LTE)", 20_000);

        b.note("server: join with the chat command; messages both ways");
        atServer(b, "chat " + CHAT_SERVER_IP + " --nick base");
        b.expect("server", "Joined Tech Village chat as base", "the server's chat command joins");
        b.await(r -> tapIn(r, "Chat", "Message Tech Village chat"), "tap the message box", 10_000);
        typeOnPhone(b, CHAT_HELLO);
        b.await(r -> tapIn(r, "Chat", "Send"), "tap Send", 5_000);
        b.expect("server", "<pat> " + CHAT_HELLO, "the server's chat shows the phone's message");
        atServer(b, CHAT_REPLY);
        b.await(r -> chatShows(r, CHAT_REPLY), "the reply shows in Chat", 15_000);
        b.await(r -> PhoneGuiScenarios.comp(r).notifications().stream().noneMatch(n -> n.appId().equals(CHAT)) ? null : "a notification while on screen",
                "control: no notification while Chat is on screen", 1_000);

        b.note("Chat to the background: home (the phone suspends it). In the phone's Terminal, allow Chat background refresh and"
                + " notifications; the server writes again; the next refresh wake fetches it and posts a notification");
        b.mutate(r -> PhoneGuiScenarios.altKey(r, PhoneGuiScenarios.HID_H), "Alt+H: home");
        b.await(r -> {
            var a = PhoneGuiScenarios.app(r, CHAT);
            return a != null && a.state() == com.example.evanscomputermod.computer.apps.AppInstance.State.SUSPENDED ? null : "Chat " + a;
        }, "Chat is suspended in the background", 10_000);
        PhoneScenarios.openTerminal(b);
        // (the phone's Terminal is 30 columns wide: pkg's answer wraps, so it is read with the rows joined)
        for (String perm : List.of("background:refresh", "notifications")) {
            PhoneScenarios.clearScreen(b);
            b.mutate(r -> {
                if (!PhoneScenarios.type(r, PhoneScenarios.holder(r), "pkg perm grant " + CHAT + " " + perm + "\n"))
                    r.fail("the phone refused its holder's typing");
            }, "in the phone's Terminal: pkg perm grant " + CHAT + " " + perm);
            b.await(r -> {
                String joined = PhoneScenarios.screen(r).replace("\n", "");
                if (joined.contains("pkg: ")) r.fail("pkg said: " + PhoneScenarios.screen(r));
                return joined.contains(CHAT + " " + perm + " allow") ? null : "phone screen:\n" + PhoneScenarios.screen(r);
            }, "pkg: " + CHAT + " " + perm + " allow", 15_000);
        }
        b.await(r -> {
            var p = PhoneGuiScenarios.phone(r).apps().permissions();
            return Boolean.TRUE.equals(p.decision(CHAT, "background:refresh")) && Boolean.TRUE.equals(p.decision(CHAT, "notifications"))
                    ? null : "decisions " + p.decisions().get(CHAT);
        }, "the phone's app manager has both allowed for Chat", 2_000);
        b.mutate(r -> PhoneGuiScenarios.phone(r).apps().background().configure(3_000, 2_000, 10),
                "refresh wakes every 3 s with a 2 s budget for the run [exception: the refresh interval compressed from minutes to"
                        + " seconds; docs/SCENARIO_EXCEPTIONS.md]");
        atServer(b, CHAT_LATER);
        b.await(r -> PhoneGuiScenarios.comp(r).notifications().stream().anyMatch(n -> n.appId().equals(CHAT) && n.body().contains(CHAT_LATER)
                        && n.title().equals("base in Tech Village chat")) ? null : "notifications " + PhoneGuiScenarios.comp(r).notifications(),
                "a refresh wake: Chat posts \"base in Tech Village chat: " + CHAT_LATER + "\"", 15_000);
        b.await(r -> PhoneGuiScenarios.onScreen(r, CHAT) == null ? null : "Chat is on screen", "Chat stayed in the background", 1_000);
        b.timeLimit(60_000);
        return b.build();
    }

    static {
        for (Scenario s : List.of(phoneCelld(), phoneAttach(), cellChat())) ALL.put(s.name, s);
    }
}
//?}
