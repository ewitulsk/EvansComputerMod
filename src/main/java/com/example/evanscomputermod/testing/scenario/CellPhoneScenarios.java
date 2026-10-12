package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.ModBlocks;
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
import com.example.evanscomputermod.wasm.WasmManager;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.phys.Vec3;

import java.nio.file.Files;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Cellular v2 phase 3, the phone side (docs/gui/CELLULAR_V2.md "As built: phase 3 phone side").
 *
 * <ul>
 *   <li>{@code cell_phone_celld} (GameTest {@code ecm_phone}): a player writes a SIM at a Card
 *       Reader/Writer with {@code simtool}, takes it out, holds a new ePhone and fits an LTE Modem and
 *       the SIM in its slot screen. The phone starts {@code celld} by itself; in the phone's Terminal
 *       {@code cell} shows the SIM (read through the {@code sim} peripheral of the SIM tray) and celld
 *       searching (no network is built here); the status bar has cellular bars and no carrier; {@code
 *       cell scan} hears nothing. Control: the SIM taken out again stops celld, and {@code cell} says
 *       celld isn't running.</li>
 *   <li>{@code cell_phone_attach} (<b>pending</b> until the network lane's {@code cored} and
 *       {@code celld-site} are merged; no GameTest yet): a carrier core computer runs {@code cored}, a
 *       cell site (computer + Advanced SDR + coax + LTE Sector Antenna) runs {@code celld-site}, and
 *       the phone attaches, gets its address from the core and pings the core's gateway. It stops at
 *       its first step with "pending" while those programs aren't in the build.</li>
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

    /** The card writer, the player's blank SIM written with simtool and taken out by hand. */
    static void writeSim(Scenario.Builder b) {
        b.note("At the card writer: a blank SIM clicked into the Card Reader/Writer, written with simtool, taken out by hand")
                .mutate(r -> r.player().use(new ItemStack(CellularContent.SIM_CARD.get()), r.abs(READER), Direction.NORTH, false),
                        "right-click the Card Reader/Writer with a blank SIM card")
                .send("pc", "simtool write " + IMSI + " " + CARRIER + " --key " + KEY)
                .expectOrFail("pc", "^card released: waiting to be taken or pulled out$", "the SIM is written (" + CARRIER + ", IMSI " + IMSI + ")",
                        "^simtool:")
                .expect("pc", PROMPT, "simtool finished")
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

    // ------------------------------------------------------------ cell_phone_attach (pending)

    static final BlockPos CORE = new BlockPos(0, 1, -6), SITE = FramedScenarios.A;
    static final String CORE_IP = "10.0.40.1", SITE_IP = "10.0.40.2";
    static final String SITE_SECRET = "5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e5e";

    /** Null when this build has the network lane's programs. */
    static String networkProgramsPresent(ScenarioRun r) {
        java.nio.file.Path bin = WasmManager.getWasmBinPath();
        for (String p : List.of("cored", "celld-site")) {
            if (bin == null || !Files.isRegularFile(bin.resolve(p + ".wasm"))) {
                r.fail("pending: " + p + " is not in this build (Cellular v2 phase 3 network lane); cell_phone_attach runs once it is merged");
                return null;
            }
        }
        return null;
    }

    static Scenario phoneAttach() {
        List<BlockPos> floor = new ArrayList<>(PlayerKit.box(-2, 0, -8, 26, 0, 3));
        List<BlockPos> cable = Scenario.Path.from(new BlockPos(0, 0, -6)).go(Direction.SOUTH, 6).blocks();
        floor.removeAll(cable);
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(cable);
        foot.addAll(FramedScenarios.station(SITE));
        foot.addAll(pedestals());
        foot.add(READER);
        var b = Scenario.builder("cell_phone_attach",
                        "PENDING (needs cored and celld-site): a carrier core and a player-built cell site; an ePhone with a modem and a SIM"
                                + " attaches through celld, gets the core's address and pings the core")
                .asPlayer()
                .realTime()
                .host("core", CORE, CORE_IP + "/24")
                .host("site", SITE, SITE_IP + "/24")
                .host("pc", WRITER, "-")
                .link("backhaul", "core", Direction.DOWN, "site", Direction.DOWN, Scenario.Path.from(new BlockPos(0, 0, -6)).go(Direction.SOUTH, 6))
                .decor(PlayerKit.decor(foot, r -> {
                    PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState());
                    PlayerKit.fill(r, pedestals(), Blocks.STONE.defaultBlockState());
                }, r -> {
                    FramedScenarios.build(r, SITE);
                    buildWriter(r);
                }, PhoneScenarios::cleanup))
                .note("PENDING: written against the program names of CELLULAR_V2.md; runs once the network lane's cored and celld-site"
                        + " are merged (their arguments and outputs may need adjusting then).")
                .await(CellPhoneScenarios::networkProgramsPresent, "cored and celld-site are in this build", 1_000)
                .note("Setup (done for you): the core computer cabled to the site computer; next to the site an Advanced SDR, coax and an"
                        + " LTE Sector Antenna pointing east; a card writer.")
                .send("core", "ifconfig eth0 " + CORE_IP + "/24")
                .expect("core", "eth0: inet " + CORE_IP.replace(".", "\\.") + "/24", "core has " + CORE_IP)
                .send("site", "ifconfig eth0 " + SITE_IP + "/24")
                .expect("site", "eth0: inet " + SITE_IP.replace(".", "\\.") + "/24", "site has " + SITE_IP);
        PlayerKit.typeFile(b, "core", "/etc/cored.db", "version 1", "carrier " + CARRIER, "pool 100.65.2.0/24", "gateway 100.65.2.1",
                "site site-a " + SITE_SECRET, "sub " + IMSI + " key=" + KEY);
        b.send("core", "cored /etc/cored.db &")
                .expect("core", "cored", "cored runs");
        PlayerKit.typeFile(b, "site", "/etc/celld-site.toml", "site_id = \"site-a\"", "secret = \"" + SITE_SECRET + "\"",
                "carrier = \"" + CARRIER + "\"", "cell_id = 1", "channel = 1", "core = \"" + CORE_IP + "\"", "sdr = \"sdr_0\"");
        b.send("site", "celld-site &")
                .expect("site", "core|registered|connected", "celld-site reaches the core");
        writeSim(b);
        b.mutate(r -> {
            PhoneScenarios.givePhone(r);
            PhoneScenarios.standAt(r.player(), r, new Vec3(20.5, 0, 0.5));
        }, "the player takes a new ePhone and walks 17 blocks east of the site's antenna")
                .await(PhoneScenarios::booted, "the phone boots", 40_000)
                .mutate(CellPhoneScenarios::fitModemAndSim, "slot screen: LTE Modem and SIM in")
                .await(CellPhoneScenarios::celldRunning, "celld runs", 15_000)
                .await(r -> statusHas(r, "\ncarrier " + CARRIER + "\n"), "the status bar shows " + CARRIER + " (attached)", 30_000);
        PhoneScenarios.openTerminal(b);
        PhoneScenarios.phoneRun(b, "cell", "^\\s+state\\s+connected", "^cell: celld is not running", "cell: connected", 15_000);
        PhoneScenarios.phoneRun(b, "ping 100.65.2.1 -n 3", "^3 packets sent, [1-3] received", "^3 packets sent, 0 received",
                "the phone pings the core's gateway over LTE", 30_000);
        b.timeLimit(120_000);
        return b.build();
    }

    static {
        for (Scenario s : List.of(phoneCelld(), phoneAttach())) ALL.put(s.name, s);
    }
}
//?}
