package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.TerminalBlock;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.card.CardContent;
import com.example.evanscomputermod.card.CardReaderBlock;
import com.example.evanscomputermod.card.CardReaderBlockEntity;
import com.example.evanscomputermod.card.SimIdentity;
import com.example.evanscomputermod.card.SimSecret;
import com.example.evanscomputermod.module.ModuleBays;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.cellular.CellularContent;
import com.example.evanscomputermod.radio.cellular.FramedModemModule;
import com.example.evanscomputermod.radio.cellular.ModemModuleItem;
import com.example.evanscomputermod.radio.cellular.SimCardItem;
import com.example.evanscomputermod.radio.compat.sable.SableShips;
import com.example.evanscomputermod.radio.framed.PhyProfile;
import com.example.evanscomputermod.radio.medium.PathTracer;
import com.example.evanscomputermod.radio.medium.WorldMediumContent;
import com.example.evanscomputermod.radio.medium.WorldRadioMedium;
import com.example.evanscomputermod.sable.SableCompat;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.phys.Vec3;

import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.WeakHashMap;
import java.util.regex.Pattern;

/**
 * Cellular v2 scenarios on real player builds (docs/gui/CELLULAR_V2.md, phase 4), spawnable
 * with {@code /ecm scenario spawn <name>} and run by the {@code ecm_cellular} GameTests. Every
 * network is made from primitives and typed into: a carrier core is a computer running
 * {@code cored} (with a Card Reader/Writer for SIMs), a cell site is a computer running
 * {@code celld-site} with an Advanced SDR, coax up a mast and a panel antenna on top
 * ({@link CellNetScenarios#buildStation}), routers are computers running the router CLI, and
 * the phones are Terminals with an LTE / mmWave Modem module whose own SIM slot holds a card
 * {@code cored sim new} wrote, running {@code celld}. Only vanilla terrain (floors, the ridge,
 * the cart's planks) is set directly; the cart of {@code cell_roaming} is moved as a Sable
 * ship by script (the standing vehicle exception), and {@code cell_lte_far} keeps its far site
 * loaded with {@code /forceload} (an operator command).
 *
 * <ul>
 *   <li>{@code cell_lte_far}: a site 300 blocks east (core, server and site built there); the
 *       phone attaches over 300 m of LTE and pings the server on the core's LAN; control: no
 *       SIM, no service.</li>
 *   <li>{@code cell_mmwave_hill}: an mmWave site and an LTE site on one core; a computer in
 *       line of sight attaches over mmWave at 256QAM, one behind an earth ridge hears nothing;
 *       its modem swapped for an LTE Modem (the SIM taken out of the old modem and clicked into
 *       the new one), it attaches through the ridge.</li>
 *   <li>{@code cell_wrong_carrier}: a SIM of carrier cell-blue hears cell-red's site but never
 *       attaches (the site sees no phone); a cell-red SIM clicked in (the blue one pops out with
 *       its key intact) attaches.</li>
 *   <li>{@code cell_roaming}: two sites behind different routers and subnets, one core; the
 *       phone on a moving cart keeps its address and an SSH session while it hands over;
 *       {@code cored handovers} logs it.</li>
 * </ul>
 */
public final class CellularScenarios {
    static final String PROMPT = PlayerKit.PROMPT;
    static final String IMSI = "00101\\d{10}";

    private CellularScenarios() {}

    // ------------------------------------------------------------ kit

    /** Per run: the SIM cards the player holds, by label; addresses noted along the way. */
    static final class Run {
        final Map<String, ItemStack> sims = new HashMap<>();
        final Map<String, String> notes = new HashMap<>();
        SableShips.Ship ship;
        Vec3 home;
        double driven;
    }

    static final Map<ScenarioRun, Run> RUNS = Collections.synchronizedMap(new WeakHashMap<>());

    static Run run(ScenarioRun r) {
        return RUNS.computeIfAbsent(r, k -> new Run());
    }

    /** A site credential (32 hex digits) the player typed into both the core and the site. */
    static String secret(String site) {
        String h = Integer.toHexString(Math.abs(site.hashCode()) | 0x10000000);
        return (h + h + h + h).substring(0, 32);
    }

    /** {@code router}, the lines (each answered by the CLI's prompt), {@code exit} back to the shell. */
    static void routerCli(Scenario.Builder b, String node, String... lines) {
        b.send(node, "router");
        b.expect(node, "^router#", node + ": the router CLI");
        for (String l : lines) {
            b.send(node, l);
            b.expect(node, "^router\\S*#", node + " router: " + l);
        }
        b.send(node, "exit");
        b.expect(node, PROMPT, node + ": back in the shell");
    }

    /** {@code cored init}, a credential per site, {@code cored start &}. */
    static void coreUp(Scenario.Builder b, String core, String carrier, String pool, String... sites) {
        b.note(core + ": the subscriber database (carrier " + carrier + ", phones' pool " + pool + "), the sites' credentials, the core");
        b.send(core, "cored init " + carrier + " " + pool);
        b.expectOrFail(core, "^cored: carrier " + Pattern.quote(carrier) + " pool " + Pattern.quote(pool), "cored writes /etc/cored.db",
                "^cored: (?!carrier)");
        for (String s : sites) {
            b.send(core, "cored site-add " + s + " " + secret(s));
            b.expect(core, "site " + Pattern.quote(s) + " added", "a credential for " + s);
        }
        b.send(core, "cored start &");
        b.expect(core, "cored: carrier " + Pattern.quote(carrier) + " pool " + Pattern.quote(pool) + ", cell0 ", "cored runs");
    }

    /** {@code celld-site config set ...}, {@code celld-site start &}, until the site is registered with the core. */
    static void siteUp(Scenario.Builder b, String site, String id, String carrier, String coreIp, int cell, PhyProfile profile, int channel,
                       String extra) {
        b.note(site + ": the site's settings (" + id + ", cell " + cell + ", " + profile.displayName + " channel " + channel + "), start it");
        b.send(site, "celld-site config set site_id " + id + " secret " + secret(id) + " carrier " + carrier + " core " + coreIp + " cell_id "
                + cell + " profile " + profile.id + " channel " + channel + extra);
        b.expectOrFail(site, "^celld-site: saved", site + "'s settings are saved", "^celld-site: (?!saved)");
        b.send(site, "celld-site start &");
        b.expectOrFail(site, "celld-site: " + Pattern.quote(id) + " cell " + cell + " \\(" + Pattern.quote(carrier) + "\\) on " + profile.id
                + " channel " + channel, "celld-site runs the radio", "^celld-site: (no framed radio|admin socket|not configured)");
        b.until(site, "celld-site status", "^link up", id + " is registered with the core over the sealed backhaul");
    }

    static CardReaderBlockEntity reader(ScenarioRun r, BlockPos rel) {
        return r.level().getBlockEntity(r.abs(rel)) instanceof CardReaderBlockEntity be ? be : null;
    }

    static void placeReader(ScenarioRun r, BlockPos rel) {
        r.player().place(CardContent.CARD_READER.get(), r.abs(rel), Direction.NORTH, s -> s.getValue(CardReaderBlock.FACING) == Direction.NORTH);
    }

    static void insertBlank(ScenarioRun r, BlockPos readerRel) {
        List<String> said = r.player().use(new ItemStack(CellularContent.SIM_CARD.get()), r.abs(readerRel), Direction.NORTH, false);
        CardReaderBlockEntity be = reader(r, readerRel);
        if (be == null || !be.hasCard()) r.fail("the Card Reader/Writer took no card: the game said " + said);
    }

    /**
     * Right-click the reader with an empty hand: the card goes into the inventory; the player keeps it as
     * {@code label} (the written card whose IMSI is {@code imsi}).
     */
    static void takeOut(ScenarioRun r, BlockPos readerRel, String label, String imsi) {
        Inventory inv = r.player().entity().getInventory();
        int was = inv.selected;
        inv.selected = 8;
        inv.setItem(8, ItemStack.EMPTY);
        r.player().useEmpty(r.abs(readerRel), Direction.NORTH, false);
        inv.selected = was;
        for (int i = 0; i < inv.getContainerSize(); i++) {
            SimIdentity id = SimCardItem.identity(inv.getItem(i));
            if (id != null && id.imsiText().equals(imsi)) {
                run(r).sims.put(label, inv.getItem(i));
                inv.setItem(i, ItemStack.EMPTY);
                r.say("§7  the player has SIM " + label + " (" + id.carrier() + ", IMSI " + imsi + ")");
                return;
            }
        }
        r.fail("taking the card out of the reader gave the player no SIM " + imsi + ": " + r.player().heard());
    }

    /** A blank SIM into the core's reader, {@code cored sim new --label L}, the card taken out by hand. */
    static void writeSim(Scenario.Builder b, String core, BlockPos readerRel, String label) {
        b.mutate(r -> insertBlank(r, readerRel), "right-click the Card Reader/Writer with a blank SIM card");
        b.send(core, "cored sim new --label " + label);
        b.expectOrFail(core, "^cored: wrote SIM IMSI " + IMSI + " \\(" + label + "\\)", "cored writes the SIM (" + label + ") and adds it to"
                + " the database", "^cored: (?!wrote)");
        b.mutate(r -> {
            String imsi = CellNetScenarios.last(r.screen(core), "^cored: wrote SIM IMSI (\\d{15}) \\(" + label + "\\)");
            if (imsi == null) r.fail("no IMSI on " + core + "'s screen");
            else takeOut(r, readerRel, label, imsi);
        }, "take the SIM out of the reader (empty hand)");
    }

    /** The terminal's face with its left bay (where {@link ScenarioPlayer#installModule} puts modules). */
    static Direction leftFace(ScenarioRun r, String node) {
        Direction facing = r.level().getBlockState(r.where(node)).getValue(TerminalBlock.FACING);
        for (Direction d : Direction.Plane.HORIZONTAL) if (ModuleBays.bayOnFace(facing, d) == ModuleBays.LEFT) return d;
        throw new IllegalStateException("no left bay face");
    }

    static FramedModemModule modemOf(ScenarioRun r, String node) {
        TerminalBlockEntity t = r.terminal(node);
        if (t == null) return null;
        for (int i = 0; i < ModuleBays.SLOTS; i++) if (t.getModuleBays().getModule(i) instanceof FramedModemModule m) return m;
        return null;
    }

    /** The player clicks an expansion card and a modem into the computer's left side. */
    static void installModem(ScenarioRun r, String node, PhyProfile tech) {
        List<String> said = r.player().installModule(r.where(node), CellularContent.modem(tech));
        TerminalBlockEntity t = r.terminal(node);
        boolean in = false;
        for (int i = 0; t != null && i < ModuleBays.SLOTS; i++) in |= t.getModuleBays().getStack(i).is(CellularContent.modem(tech));
        if (!in) throw new IllegalStateException(tech.displayName + " Modem not installed in " + node + ": " + said);
        r.say("§7  " + tech.displayName + " Modem clicked into " + node + " (expansion card first, left side)");
    }

    /** Right-click the modem's bay (left side, upper slot) with the SIM {@code label}: it goes into the modem's own slot. */
    static void insertSim(ScenarioRun r, String node, String label) {
        ItemStack sim = run(r).sims.remove(label);
        if (sim == null) {
            r.fail("the player has no SIM " + label);
            return;
        }
        BlockPos p = r.where(node);
        Direction left = leftFace(r, node);
        List<String> said = r.player().use(sim, p, left, ScenarioPlayer.faceCentre(p, left).add(0, 0.25, 0), false);
        FramedModemModule m = modemOf(r, node);
        SimIdentity want = SimCardItem.identity(sim), got = m == null ? null : SimCardItem.identity(m.ownSim());
        if (got == null || !got.equals(want)) {
            r.fail("SIM " + label + " not taken by " + node + "'s modem: the game said " + said);
            return;
        }
        r.say("§7  SIM " + label + " (" + want.carrier() + " " + want.imsiText() + ") clicked into " + node + "'s modem: " + String.join(" ", said));
    }

    /** {@code celld &} on a Terminal, then {@code cell} until it is connected. */
    static void celldConnects(Scenario.Builder b, String node, String what) {
        b.send(node, "celld &");
        b.until(node, "cell status", "^\\s+state\\s+connected", what);
    }

    /** The phone's address as {@code cell} shows it ({@code address   100.65.x.y/24 via ...}). */
    static String cellAddress(ScenarioRun r, String node) {
        return CellNetScenarios.last(r.screen(node), "^\\s+address\\s+(100\\.65\\.\\d+\\.\\d+)/\\d+ via");
    }

    static double distance(Pose a, BlockPos absBlock) {
        double dx = a.x() - (absBlock.getX() + 0.5), dy = a.y() - (absBlock.getY() + 0.5), dz = a.z() - (absBlock.getZ() + 0.5);
        return Math.sqrt(dx * dx + dy * dy + dz * dz);
    }

    static void logScreen(ScenarioRun r, String node) {
        EvansComputerMod.LOGGER.info("[cellular] {} screen:\n{}", node, r.screen(node));
    }

    // ------------------------------------------------------------ cell_lte_far

    /** The far site: 300 blocks east of the phone. */
    public static final int FAR = 300;
    static final BlockPos FAR_UE = new BlockPos(0, 1, 0);
    static final BlockPos FAR_CORE = new BlockPos(FAR, 1, -4), FAR_READER = FAR_CORE.east(), FAR_SERVER = new BlockPos(FAR + 4, 1, -4),
            FAR_SITE = new BlockPos(FAR, 1, 0);
    /** Backhaul: core bottom to site bottom; LAN: core top to server top. */
    static final List<BlockPos> FAR_BACKHAUL = Scenario.Path.from(new BlockPos(FAR, 0, -4)).go(Direction.SOUTH, 4).blocks();
    static final List<BlockPos> FAR_LAN = Scenario.Path.from(new BlockPos(FAR, 2, -4)).go(Direction.EAST, 4).blocks();
    static final String FAR_CARRIER = "cell-far", FAR_POOL = "100.65.10.0/24";

    static List<BlockPos> farBox() {
        return PlayerKit.box(FAR - 5, 0, -6, FAR + 6, 6, 2);
    }

    /**
     * {@code cell_lte_far}: here the phone, a computer with an LTE Modem; 300 blocks east (kept loaded with
     * /forceload) a core (with its Card Reader/Writer), a server on its LAN and a cell site whose LTE panel faces
     * west towards the phone. Everything at the far end is built and typed there by the player.
     */
    public static Scenario lteFar() {
        List<BlockPos> floor = PlayerKit.box(-3, 0, -2, 3, 0, 2);
        var b = Scenario.builder("cell_lte_far",
                        "a computer with an LTE Modem attaches to a player-built cell site 300 blocks away (cored + celld-site there) and"
                                + " pings a server on the core's LAN; without a SIM it has no service")
                .asPlayer()
                .realTime()
                .host("ue", FAR_UE, "-")
                .remoteHost("core", FAR_CORE, "-")
                .remoteHost("server", FAR_SERVER, "-")
                .remoteHost("site", FAR_SITE, "-")
                .decor(new Scenario.Decor() {
                    @Override public List<BlockPos> footprint() {
                        return floor;
                    }

                    @Override public void terrain(ScenarioRun r) {
                        BlockPos lo = r.abs(new BlockPos(FAR - 5, 0, -6)), hi = r.abs(new BlockPos(FAR + 6, 0, 2));
                        r.say("§7  " + String.join(" ", r.player().command(String.format(Locale.ROOT, "/forceload add %d %d %d %d",
                                lo.getX(), lo.getZ(), hi.getX(), hi.getZ()))));
                        for (BlockPos p : farBox()) r.level().setBlock(r.abs(p), Blocks.AIR.defaultBlockState(), 3);
                        for (BlockPos p : PlayerKit.box(FAR - 5, 0, -6, FAR + 6, 0, 2))
                            if (!FAR_BACKHAUL.contains(p)) r.level().setBlock(r.abs(p), Blocks.SMOOTH_STONE.defaultBlockState(), 3);
                        PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState());
                    }

                    @Override public void build(ScenarioRun r) {
                        for (BlockPos p : FAR_BACKHAUL) r.player().place(ModBlocks.NETWORK_CABLE.get(), r.abs(p), Direction.UP);
                        for (BlockPos p : FAR_LAN) r.player().place(ModBlocks.NETWORK_CABLE.get(), r.abs(p), Direction.UP);
                        placeReader(r, FAR_READER);
                        CellNetScenarios.buildStation(r, FAR_SITE, -1);
                        installModem(r, "ue", PhyProfile.LTE);
                    }

                    @Override public void clear(ScenarioRun r) {
                        for (BlockPos p : farBox()) r.level().setBlock(r.abs(p), Blocks.AIR.defaultBlockState(), 3);
                        BlockPos lo = r.abs(new BlockPos(FAR - 5, 0, -6)), hi = r.abs(new BlockPos(FAR + 6, 0, 2));
                        r.player().command(String.format(Locale.ROOT, "/forceload remove %d %d %d %d", lo.getX(), lo.getZ(), hi.getX(), hi.getZ()));
                    }
                })
                .note("Built for you: here a computer with an LTE Modem (no SIM). 300 blocks east (kept loaded with /forceload): a core"
                        + " computer with a Card Reader/Writer, a server cabled to its top, and a cell site cabled to its bottom (an Advanced"
                        + " SDR, coax up a two-block mast and an LTE Sector Antenna facing west, towards the phone).");
        b.send("server", "ifconfig eth1 10.0.31.10/24");
        b.send("site", "ifconfig eth0 10.0.30.2/24");
        b.send("ue", "celld &");
        b.expect("server", "eth1: inet 10\\.0\\.31\\.10/24", "the server has 10.0.31.10");
        b.send("server", "ip route add " + FAR_POOL + " via 10.0.31.1 dev eth1");
        b.expect("server", "^Route 100\\.65\\.10\\.0/24 added", "the server reaches the phones through the core");
        b.expect("site", "eth0: inet 10\\.0\\.30\\.2/24", "the site has 10.0.30.2");
        b.note("Control: the phone's modem has no SIM; celld has nothing to attach with");
        b.until("ue", "cell status", "^\\s+state\\s+no_sim", "cell: no SIM card");
        b.note("core: addresses and IP forwarding in its router CLI (the phones' traffic leaves through it)");
        routerCli(b, "core", "configure terminal", "interface eth0", "ip address 10.0.30.1/24", "exit", "interface eth1",
                "ip address 10.0.31.1/24", "exit", "ip routing", "end");
        coreUp(b, "core", FAR_CARRIER, FAR_POOL, "site-far");
        siteUp(b, "site", "site-far", FAR_CARRIER, "10.0.30.1", 1, PhyProfile.LTE, 2, "");
        b.note("At the core: a SIM. Walk back the 300 blocks and right-click the phone's modem bay with it");
        writeSim(b, "core", FAR_READER, "far");
        b.mutate(r -> insertSim(r, "ue", "far"), "right-click the phone's LTE Modem bay with the SIM");
        b.until("ue", "cell status", "^\\s+state\\s+connected - on cell-far cell 1", "celld finds the SIM and attaches to cell-far over LTE");
        b.await(r -> {
            FramedModemModule m = modemOf(r, "ue");
            if (m == null || m.pose() == null) return "no modem pose";
            double d = distance(m.pose(), r.abs(CellNetScenarios.panel(FAR_SITE, -1)));
            r.say(String.format(Locale.ROOT, "§7  the site's antenna is %.1f m from the phone's modem", d));
            if (d < 290) r.fail("the site is only " + d + " m away");
            return null;
        }, "the site's antenna is ~300 m away", 2_000);
        b.until("ue", "ping 10.0.31.10 -n 1", "^1 packets sent, 1 received", "the phone reaches the server through the site and the core");
        b.send("ue", "ping 10.0.31.10 -n 5");
        b.expectOrFail("ue", "^5 packets sent, [45] received", "at least 4 of 5 pings over 300 m of LTE", "^5 packets sent, [0-3] received");
        b.send("core", "cored status");
        b.expectOrFail("core", "^session \\d+ " + IMSI + " 100\\.65\\.10\\.\\d+ active site-far 1", "cored lists the phone's session at site-far",
                "^sessions 0");
        b.timeLimit(60_000);
        return b.build();
    }

    // ------------------------------------------------------------ cell_mmwave_hill

    static final BlockPos HILL_CORE = new BlockPos(0, 1, -3), HILL_READER = HILL_CORE.east();
    /** The mmWave site grows west of its computer, the LTE site east; both panels face south. */
    static final BlockPos HILL_MM = new BlockPos(-2, 1, 0), HILL_LTE = new BlockPos(2, 1, 0);
    static final BlockPos HILL_LOS = new BlockPos(-5, 1, 28), HILL_HIDDEN = new BlockPos(6, 1, 28);
    static final List<BlockPos> RIDGE = PlayerKit.box(3, 1, 22, 10, 6, 24);
    static final String HILL_CARRIER = "cell-hill", HILL_POOL = "100.65.11.0/24";

    /**
     * {@code cell_mmwave_hill}: a core with two sites (an mmWave Array site, cell 1, and an LTE Sector site, cell 2,
     * both facing south). 28 blocks south: "los" (mmWave Modem) in the open and "hidden" (mmWave Modem) behind a
     * three-block-thick earth ridge taller than the line between the antennas.
     */
    public static Scenario mmwaveHill() {
        List<BlockPos> floor = new ArrayList<>(PlayerKit.box(-9, 0, -5, 11, 0, 30));
        List<BlockPos> mmLink = Scenario.Path.from(new BlockPos(0, 0, -3)).go(Direction.SOUTH, 3).go(Direction.WEST, 2).blocks();
        List<BlockPos> lteLink = Scenario.Path.from(new BlockPos(0, 2, -3)).go(Direction.SOUTH, 3).go(Direction.EAST, 2).blocks();
        floor.removeAll(mmLink);
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(RIDGE);
        foot.addAll(CellNetScenarios.station(HILL_MM, -1));
        foot.addAll(CellNetScenarios.station(HILL_LTE, 1));
        foot.add(HILL_READER);
        var b = Scenario.builder("cell_mmwave_hill",
                        "mmWave works in line of sight and fails behind an earth ridge (path-loss check); the computer behind the ridge,"
                                + " its modem swapped for an LTE Modem, attaches to the LTE site through it")
                .asPlayer()
                .realTime()
                .host("core", HILL_CORE, "-")
                .host("mm", HILL_MM, "-")
                .host("lte", HILL_LTE, "-")
                .host("los", HILL_LOS, "-")
                .host("hidden", HILL_HIDDEN, "-")
                .link("mm_backhaul", "core", Direction.DOWN, "mm", Direction.DOWN, Scenario.Path.from(new BlockPos(0, 0, -3))
                        .go(Direction.SOUTH, 3).go(Direction.WEST, 2))
                .link("lte_backhaul", "core", Direction.UP, "lte", Direction.UP, Scenario.Path.from(new BlockPos(0, 2, -3))
                        .go(Direction.SOUTH, 3).go(Direction.EAST, 2))
                .decor(PlayerKit.decor(foot, r -> {
                    PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState());
                    PlayerKit.fill(r, RIDGE, Blocks.DIRT.defaultBlockState());
                    for (BlockPos p : PlayerKit.box(3, 6, 22, 10, 6, 24)) r.level().setBlock(r.abs(p), Blocks.GRASS_BLOCK.defaultBlockState(), 3);
                }, r -> {
                    placeReader(r, HILL_READER);
                    CellNetScenarios.buildStation(r, HILL_MM, -1, Direction.SOUTH, CellularContent.MMWAVE_ANTENNA.get());
                    CellNetScenarios.buildStation(r, HILL_LTE, 1, Direction.SOUTH, CellularContent.LTE_ANTENNA.get());
                    installModem(r, "los", PhyProfile.MMWAVE);
                    installModem(r, "hidden", PhyProfile.MMWAVE);
                }, null))
                .note("Built for you: a core with a Card Reader/Writer, cabled (bottom) to an mmWave site (Advanced SDR, coax mast, mmWave"
                        + " Array Antenna facing south) and (top) to an LTE site (LTE Sector Antenna facing south). 28 blocks south: los in"
                        + " the open and hidden behind an earth ridge, each with an mmWave Modem.");
        b.send("mm", "ifconfig eth0 10.0.50.2/24");
        b.send("lte", "ifconfig eth1 10.0.51.2/24");
        b.send("los", "cell status");
        b.send("hidden", "cell status");
        b.send("core", "ifconfig eth0 10.0.50.1/24");
        b.expect("core", "eth0: inet 10\\.0\\.50\\.1/24", "the core has 10.0.50.1 towards the mmWave site");
        b.send("core", "ifconfig eth1 10.0.51.1/24");
        b.expect("core", "eth1: inet 10\\.0\\.51\\.1/24", "and 10.0.51.1 towards the LTE site");
        coreUp(b, "core", HILL_CARRIER, HILL_POOL, "site-mm", "site-lte");
        siteUp(b, "mm", "site-mm", HILL_CARRIER, "10.0.50.1", 1, PhyProfile.MMWAVE, 1, " power 37");
        siteUp(b, "lte", "site-lte", HILL_CARRIER, "10.0.51.1", 2, PhyProfile.LTE, 4, "");
        b.note("SIMs for los and hidden, clicked into their modems' bays; celld on both");
        writeSim(b, "core", HILL_READER, "los");
        writeSim(b, "core", HILL_READER, "hidden");
        b.mutate(r -> {
            insertSim(r, "los", "los");
            insertSim(r, "hidden", "hidden");
        }, "right-click each mmWave Modem's bay with its SIM");
        b.send("hidden", "celld &");
        b.note("Line of sight: mmWave attaches at a 256QAM rate");
        celldConnects(b, "los", "los attaches to cell 1 over mmWave");
        b.expect("los", "^\\s+signal\\s+-?\\d+\\.\\d dBm, SINR -?\\d+\\.\\d dB, CQI 1[6-8]\\b", "a 256QAM downlink (CQI 16-18) in line of sight",
                "^\\s+signal\\s+.*CQI ([0-9]|1[0-5])\\b");
        b.until("los", "ping 100.65.11.1 -n 1", "^1 packets sent, 1 received", "los reaches the core over mmWave");
        b.note("Control: behind the ridge, mmWave hears nothing at all");
        b.mutate(CellularScenarios::checkHillPaths, "path loss: mmWave panel -> los and -> hidden at 28 GHz, LTE panel -> hidden at 800 MHz");
        b.send("hidden", "cell scan");
        b.expectOrFail("hidden", "^no cells heard", "the hidden computer hears no cell", "^cell-hill\\s");
        b.send("hidden", "cell status");
        b.expectOrFail("hidden", "^\\s+state\\s+searching", "not attached behind the ridge", "^\\s+state\\s+connected");
        b.note("Swap hidden's modem: stop celld (ps, kill), sneak-right-click the bay (the modem comes out), sneak-use the loose modem"
                + " (its SIM comes out), click an LTE Modem in and the SIM into it, start celld again");
        b.send("hidden", "ps");
        b.expect("hidden", "^\\s*\\d+\\s+\\S+\\s+celld(\\.wasm)?\\s*$", "ps lists celld");
        b.sendFn("hidden", r -> "kill " + CellNetScenarios.last(r.screen("hidden"), "^\\s*(\\d+)\\s+\\S+\\s+celld(?:\\.wasm)?\\s*$"),
                "kill <celld's pid>");
        b.expect("hidden", "^Process killed\\.", "celld stopped");
        b.mutate(CellularScenarios::swapToLte, "hidden: mmWave Modem out, its SIM out of it, LTE Modem in, the SIM into the LTE Modem");
        celldConnects(b, "hidden", "LTE attaches to cell 2 through the ridge");
        b.expect("hidden", "^\\s+cell\\s+cell-hill cell 2, channel 4", "on the LTE site (cell 2, channel 4)");
        b.until("hidden", "ping 100.65.11.1 -n 1", "^1 packets sent, 1 received", "hidden reaches the core over LTE through the ridge");
        b.send("core", "cored sessions");
        b.expect("core", "^session \\d+ " + IMSI + " 100\\.65\\.11\\.\\d+ active site-lte 2", "cored: hidden's session is at site-lte");
        b.timeLimit(60_000);
        return b.build();
    }

    /** Sneak-right-click the bay (modem out), sneak-use it in the air (SIM out), LTE Modem in, the SIM into it. */
    static void swapToLte(ScenarioRun r) {
        BlockPos p = r.where("hidden");
        Direction left = leftFace(r, "hidden");
        Inventory inv = r.player().entity().getInventory();
        inv.clearContent();
        inv.selected = 8;   // an empty hand in hotbar slot 9: the modem comes out into the first free slot
        r.player().use(ItemStack.EMPTY, p, left, ScenarioPlayer.faceCentre(p, left).add(0, 0.25, 0), true);
        inv.selected = 0;
        if (modemOf(r, "hidden") != null) {
            r.fail("the mmWave Modem is still in hidden");
            return;
        }
        int slot = -1;
        for (int i = 0; i < inv.getContainerSize(); i++) if (inv.getItem(i).getItem() instanceof ModemModuleItem) slot = i;
        if (slot < 0) {
            r.fail("the modem didn't come out into the inventory");
            return;
        }
        ItemStack old = inv.getItem(slot);
        inv.setItem(slot, ItemStack.EMPTY);
        List<String> said = r.player().useInAir(old, true);
        ItemStack sim = ItemStack.EMPTY;
        for (int i = 0; i < inv.getContainerSize(); i++) {
            if (SimCardItem.isIssued(inv.getItem(i))) {
                sim = inv.getItem(i);
                inv.setItem(i, ItemStack.EMPTY);
            }
        }
        if (sim.isEmpty() || SimCardItem.sim(sim) == null) {
            r.fail("sneak-using the loose mmWave Modem gave no SIM (with its key): " + said);
            return;
        }
        r.say("§7  the mmWave Modem came out; sneak-used in the air it gave back its SIM: " + String.join(" ", said));
        installModem(r, "hidden", PhyProfile.LTE);
        run(r).sims.put("hidden", sim);
        insertSim(r, "hidden", "hidden");
    }

    /** The traced paths: los clear at 28 GHz, hidden blocked (> 100 dB of earth); LTE through the ridge costs tens of dB, not hundreds. */
    static void checkHillPaths(ScenarioRun r) {
        WorldRadioMedium m = WorldMediumContent.medium();
        var w = WorldMediumContent.rfWorld(r.level());
        FramedModemModule l = modemOf(r, "los"), h = modemOf(r, "hidden");
        if (m == null || w == null || l == null || h == null || l.pose() == null || h.pose() == null) {
            r.fail("no in-world medium or modem poses");
            return;
        }
        var c = new WorldRadioMedium.Conditions(r.level().getDayTime(), r.level().isThundering(), true);
        Pose los = l.pose(), hid = h.pose();
        BlockPos mmAt = r.abs(CellNetScenarios.panel(HILL_MM, -1)), lteAt = r.abs(CellNetScenarios.panel(HILL_LTE, 1));
        Pose mm = Pose.at(los.dimension(), mmAt.getX() + 0.5, mmAt.getY() + 0.5, mmAt.getZ() + 0.5);
        Pose lte = Pose.at(los.dimension(), lteAt.getX() + 0.5, lteAt.getY() + 0.5, lteAt.getZ() + 0.5);
        double f28 = PhyProfile.MMWAVE.downlink(1).centerHz(), f8 = PhyProfile.LTE.downlink(4).centerHz();
        PathTracer.Result a = m.traceNow(w, mm, los, f28, c), bb = m.traceNow(w, mm, hid, f28, c), t = m.traceNow(w, lte, hid, f8, c);
        r.say(String.format(Locale.ROOT, "§7  28 GHz site->los: %.1f m, %.1f dB (walls %.1f, diffraction %.1f, %s)", a.distanceM(), a.totalDb(),
                a.obstructionDb(), a.diffractionDb(), a.lineOfSight() ? "LOS" : "NLOS"));
        r.say(String.format(Locale.ROOT, "§7  28 GHz site->hidden: %.1f m, %.1f dB (walls %.1f, diffraction %.1f, %s)", bb.distanceM(),
                bb.totalDb(), bb.obstructionDb(), bb.diffractionDb(), bb.lineOfSight() ? "LOS" : "NLOS"));
        r.say(String.format(Locale.ROOT, "§7  800 MHz LTE site->hidden: %.1f m, %.1f dB (walls %.1f, diffraction %.1f)", t.distanceM(), t.totalDb(),
                t.obstructionDb(), t.diffractionDb()));
        if (!a.lineOfSight() || a.obstructionDb() > 3) r.fail("site -> los is not a clear line of sight: " + a.obstructionDb() + " dB of walls");
        else if (bb.totalDb() - a.totalDb() < 100) r.fail("the ridge only costs " + (bb.totalDb() - a.totalDb()) + " dB at 28 GHz");
        else if (t.totalDb() - bb.totalDb() > -50) r.fail("LTE through the ridge isn't far better than mmWave: " + t.totalDb() + " vs " + bb.totalDb());
    }

    // ------------------------------------------------------------ cell_wrong_carrier

    static final BlockPos WC_CORE = new BlockPos(0, 1, -4), WC_READER = WC_CORE.east(), WC_SITE = new BlockPos(0, 1, 0);
    static final BlockPos WC_PHONE = new BlockPos(0, 1, 12);
    static final String WC_POOL = "100.65.12.0/24", BLUE_IMSI = "001020000000042";

    /**
     * {@code cell_wrong_carrier}: carrier cell-red's core and site (an LTE panel facing south, channel 5); the phone, 12
     * blocks south, has an LTE Modem with a cell-blue SIM (written with simtool at the same card writer: a carrier this
     * core knows nothing about). It hears the red site but never attaches, and the site sees no phone. A cell-red SIM
     * clicked into the modem pops the blue one out (with its key) and attaches.
     */
    public static Scenario wrongCarrier() {
        List<BlockPos> floor = new ArrayList<>(PlayerKit.box(-3, 0, -6, 6, 0, 14));
        List<BlockPos> cable = Scenario.Path.from(new BlockPos(0, 0, -4)).go(Direction.SOUTH, 4).blocks();
        floor.removeAll(cable);
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(CellNetScenarios.station(WC_SITE, 1));
        foot.add(WC_READER);
        var b = Scenario.builder("cell_wrong_carrier",
                        "a SIM of carrier cell-blue hears carrier cell-red's site but never attaches to it (the site sees no phone);"
                                + " a cell-red SIM written by the red core attaches")
                .asPlayer()
                .realTime()
                .host("core", WC_CORE, "-")
                .host("site", WC_SITE, "-")
                .host("phone", WC_PHONE, "-")
                .link("backhaul", "core", Direction.DOWN, "site", Direction.DOWN, Scenario.Path.from(new BlockPos(0, 0, -4)).go(Direction.SOUTH, 4))
                .decor(PlayerKit.decor(foot, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()), r -> {
                    placeReader(r, WC_READER);
                    CellNetScenarios.buildStation(r, WC_SITE, 1, Direction.SOUTH, CellularContent.LTE_ANTENNA.get());
                    installModem(r, "phone", PhyProfile.LTE);
                }, null))
                .note("Built for you: cell-red's core (with a Card Reader/Writer) cabled to its site (Advanced SDR, coax mast, LTE Sector"
                        + " Antenna facing south); 12 blocks south a computer with an LTE Modem.");
        b.send("site", "ifconfig eth0 10.0.52.2/24");
        b.send("phone", "cell status");
        b.send("core", "ifconfig eth0 10.0.52.1/24");
        b.expect("core", "eth0: inet 10\\.0\\.52\\.1/24", "the core has 10.0.52.1");
        coreUp(b, "core", "cell-red", WC_POOL, "site-red");
        siteUp(b, "site", "site-red", "cell-red", "10.0.52.1", 1, PhyProfile.LTE, 5, "");
        b.note("At the same card writer, a SIM for carrier cell-blue, written with simtool (cell-red's core knows nothing about it)");
        b.mutate(r -> insertBlank(r, WC_READER), "right-click the Card Reader/Writer with a blank SIM card");
        b.send("core", "simtool write " + BLUE_IMSI + " cell-blue --random");
        b.expectOrFail("core", "^card released: waiting to be taken or pulled out$", "the cell-blue SIM is written", "^simtool:");
        b.mutate(r -> {
            String key = CellNetScenarios.last(r.screen("core"), "^key ([0-9a-f]{32})");
            if (key == null) r.fail("simtool printed no key");
            run(r).notes.put("blue_key", key);
            takeOut(r, WC_READER, "blue", BLUE_IMSI);
        }, "take the blue SIM out (empty hand)");
        b.mutate(r -> insertSim(r, "phone", "blue"), "right-click the phone's LTE Modem bay with the cell-blue SIM");
        b.send("phone", "celld &");
        b.note("Control: the blue SIM hears the red site but doesn't attach");
        b.until("phone", "cell scan", "^cell-red\\s+1\\s+lte\\s+5\\s.*other carrier", "cell scan lists cell-red's cell 1 on LTE channel 5 (other carrier)");
        b.waitMs(2_000, "time for celld to have tried, if it were going to");
        b.send("phone", "cell status");
        b.expectOrFail("phone", "^\\s+state\\s+searching - no service \\(heard cell-red\\), looking for a cell-blue cell",
                "still searching for cell-blue (no service, heard cell-red)", "^\\s+state\\s+(connected|attaching)");
        b.send("site", "celld-site status");
        b.expectOrFail("site", "^phones 0$", "the red site has no phone", "^phones [1-9]");
        b.send("core", "cored log");
        b.expect("core", PROMPT, "the core's log");
        b.await(r -> {
            String scr = r.screen("core");
            String log = scr.substring(Math.max(0, scr.lastIndexOf("cored log")));
            if (log.contains(BLUE_IMSI)) r.fail("the core's log mentions the blue SIM: " + log);
            return null;
        }, "the core never heard of the blue SIM", 1_000);
        b.note("At the red core: a cell-red SIM; right-click the modem's bay with it (the blue SIM comes back out)");
        b.mutate(r -> r.player().entity().getInventory().clearContent(), "empty pockets");
        writeSim(b, "core", WC_READER, "red");
        b.mutate(r -> insertSim(r, "phone", "red"), "right-click the phone's LTE Modem bay with the cell-red SIM");
        b.await(CellularScenarios::blueCameBack, "the blue SIM is back in the player's inventory, its key intact", 2_000);
        b.until("phone", "cell status", "^\\s+state\\s+connected - on cell-red cell 1", "celld sees the new SIM and attaches to cell-red");
        b.until("phone", "ping 100.65.12.1 -n 1", "^1 packets sent, 1 received", "the phone reaches the red core");
        b.timeLimit(60_000);
        return b.build();
    }

    static String blueCameBack(ScenarioRun r) {
        Inventory inv = r.player().entity().getInventory();
        for (int i = 0; i < inv.getContainerSize(); i++) {
            SimIdentity id = SimCardItem.identity(inv.getItem(i));
            if (id == null || !id.imsiText().equals(BLUE_IMSI)) continue;
            SimSecret s = SimCardItem.secret(inv.getItem(i));
            if (s == null || !s.known() || !s.keyHex().equals(run(r).notes.get("blue_key"))) {
                r.fail("the blue SIM came back without its key");
                return "failed";
            }
            r.say("§7  the blue SIM popped out of the modem with the key simtool wrote");
            return null;
        }
        return "no blue SIM in the inventory";
    }

    // ------------------------------------------------------------ cell_roaming

    /** Site A (x 0) grows east, site B (x 40) west; each behind its own router; the core and the server in the middle. */
    static final BlockPos RO_A = new BlockPos(0, 1, 0), RO_B = new BlockPos(40, 1, 0), RO_RA = new BlockPos(0, 1, -4), RO_RB = new BlockPos(40, 1, -4);
    static final BlockPos RO_CORE = new BlockPos(20, 1, -4), RO_READER = RO_CORE.east(), RO_SERVER = new BlockPos(20, 1, -8);
    /**
     * The cart rides along the line between the sites at the panels' height (a raised cart: sector panels have a narrow,
     * down-tilted vertical beam, so a phone below a mast close to it hears the far site nearly as well), from in front of
     * A's panel to B's side of the midpoint; the near site stays well ahead at both ends.
     */
    static final BlockPos RO_PHONE = new BlockPos(6, 4, 2);
    static final List<BlockPos> RO_CART = PlayerKit.box(5, 3, 1, 7, 3, 3);
    static final String RO_POOL = "100.65.13.0/24";
    /** How far east the cart drives, blocks; and how fast (blocks per tick). */
    static final double RO_DRIVE = 28, RO_SPEED = 0.25;

    /**
     * {@code cell_roaming}: sites A and B (cell 1 and 2, LTE channel 2) of one carrier, each behind its own router
     * computer in its own subnet (10.0.61.0/24 behind router ra, 10.0.63.0/24 behind rb); the core (cored, IP
     * forwarding, static routes to both site subnets) between them with a server (sshd) on its LAN. The phone, a
     * computer with an LTE Modem on a plank cart in front of A, attaches, SSHes into the server; the cart drives
     * east, the phone hands over to B with the same address and the same SSH session; {@code cored handovers} logs it.
     */
    public static Scenario roaming() {
        List<BlockPos> rbLink = Scenario.Path.from(new BlockPos(40, 2, -4)).go(Direction.WEST, 20).blocks();
        List<BlockPos> lanLink = Scenario.Path.from(new BlockPos(20, 0, -8)).go(Direction.SOUTH, 4).blocks();
        List<BlockPos> aLink = Scenario.Path.from(new BlockPos(0, 0, 0)).go(Direction.NORTH, 4).blocks();
        List<BlockPos> bLink = Scenario.Path.from(new BlockPos(40, 0, 0)).go(Direction.NORTH, 4).blocks();
        List<BlockPos> floor = new ArrayList<>(PlayerKit.box(-2, 0, -10, 42, 0, 6));
        floor.removeAll(lanLink);
        floor.removeAll(aLink);
        floor.removeAll(bLink);
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(RO_CART);
        foot.addAll(CellNetScenarios.station(RO_A, 1));
        foot.addAll(CellNetScenarios.station(RO_B, -1));
        foot.add(RO_READER);
        var b = Scenario.builder("cell_roaming",
                        "two cell sites behind different routers and subnets, one core: a phone on a moving cart keeps its address and an"
                                + " SSH session while it hands over from site A to site B (the cart's movement is scripted)")
                .asPlayer()
                .realTime()
                .host("site_a", RO_A, "-")
                .host("site_b", RO_B, "-")
                .host("ra", RO_RA, "-")
                .host("rb", RO_RB, "-")
                .host("core", RO_CORE, "-")
                .host("server", RO_SERVER, "-")
                .host("phone", RO_PHONE, "-")
                .link("a", "site_a", Direction.DOWN, "ra", Direction.DOWN, Scenario.Path.from(new BlockPos(0, 0, 0)).go(Direction.NORTH, 4))
                .link("b", "site_b", Direction.DOWN, "rb", Direction.DOWN, Scenario.Path.from(new BlockPos(40, 0, 0)).go(Direction.NORTH, 4))
                .link("ra_core", "ra", Direction.EAST, "core", Direction.WEST, Scenario.Path.from(new BlockPos(1, 1, -4)).go(Direction.EAST, 18))
                .link("rb_core", "rb", Direction.UP, "core", Direction.UP, Scenario.Path.from(new BlockPos(40, 2, -4)).go(Direction.WEST, 20))
                .link("lan", "server", Direction.DOWN, "core", Direction.DOWN, Scenario.Path.from(new BlockPos(20, 0, -8)).go(Direction.SOUTH, 4))
                .decor(new Scenario.Decor() {
                    @Override public List<BlockPos> footprint() {
                        return foot;
                    }

                    @Override public void terrain(ScenarioRun r) {
                        PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState());
                        PlayerKit.fill(r, RO_CART, Blocks.SPRUCE_PLANKS.defaultBlockState());
                    }

                    @Override public void build(ScenarioRun r) {
                        placeReader(r, RO_READER);
                        CellNetScenarios.buildStation(r, RO_A, 1);
                        CellNetScenarios.buildStation(r, RO_B, -1);
                        installModem(r, "phone", PhyProfile.LTE);
                    }

                    @Override public void clear(ScenarioRun r) {
                        Run s = RUNS.remove(r);
                        if (s != null && s.ship != null && !s.ship.removed()) {
                            SableShips.release(s.ship);
                            SableShips.discard(s.ship);
                        }
                    }
                })
                .note("Built for you: site A (x 0) and site B (x 40), each a computer with an Advanced SDR, a coax mast and an LTE Sector"
                        + " Antenna facing the other; each cabled to its own router computer (ra, rb), both routers cabled to the core in the"
                        + " middle (Card Reader/Writer on it), a server on the core's LAN; the phone, a computer with an LTE Modem, on a plank"
                        + " cart in front of A.");
        String coreA = b.eth("core", Direction.WEST), coreB = b.eth("core", Direction.UP), coreLan = b.eth("core", Direction.DOWN);
        b.send("server", "ifconfig eth0 10.0.64.10/24");
        b.send("site_a", "ifconfig eth0 10.0.61.2/24");
        b.send("site_b", "ifconfig eth0 10.0.63.2/24");
        b.send("phone", "cell status");
        b.send("ra", "ifconfig");
        b.send("rb", "ifconfig");
        b.expect("server", "eth0: inet 10\\.0\\.64\\.10/24", "the server has 10.0.64.10");
        b.send("server", "ip route add " + RO_POOL + " via 10.0.64.1 dev eth0");
        b.expect("server", "^Route 100\\.65\\.13\\.0/24 added", "the server reaches the phones through the core");
        b.send("server", "sshd &");
        b.expect("server", "sshd: starting on port 22", "sshd listening on the server");
        b.note("Each site's subnet is behind its own router: a default route on the site, addresses and forwarding on the router");
        for (String[] s : new String[][] {{"site_a", "10.0.61.1", "10\\.0\\.61\\.2"}, {"site_b", "10.0.63.1", "10\\.0\\.63\\.2"}}) {
            b.expect(s[0], "eth0: inet " + s[2] + "/24", s[0] + " has its address");
            b.send(s[0], "ip route add default via " + s[1] + " dev eth0");
            b.expect(s[0], "^Default route added", s[0] + ": default route via its router");
        }
        routerCli(b, "ra", "configure terminal", "interface " + b.eth("ra", Direction.EAST), "ip address 10.0.60.2/24", "exit", "interface eth0",
                "ip address 10.0.61.1/24", "exit", "ip routing", "end");
        routerCli(b, "rb", "configure terminal", "interface eth1", "ip address 10.0.62.2/24", "exit", "interface eth0",
                "ip address 10.0.63.1/24", "exit", "ip routing", "end");
        b.note("core: its three interfaces, forwarding, a static route to each site's subnet");
        routerCli(b, "core", "configure terminal", "interface " + coreA, "ip address 10.0.60.1/24", "exit", "interface " + coreB,
                "ip address 10.0.62.1/24", "exit", "interface " + coreLan, "ip address 10.0.64.1/24", "exit", "ip routing",
                "ip route 10.0.61.0/24 10.0.60.2 " + coreA, "ip route 10.0.63.0/24 10.0.62.2 " + coreB, "end");
        coreUp(b, "core", "cell-red", RO_POOL, "site-a", "site-b");
        siteUp(b, "site_a", "site-a", "cell-red", "10.0.60.1", 1, PhyProfile.LTE, 2, " neighbours 2:2");
        siteUp(b, "site_b", "site-b", "cell-red", "10.0.62.1", 2, PhyProfile.LTE, 2, " neighbours 1:2");
        b.send("core", "cored sites");
        b.expect("core", "^site site-b 1 10\\.0\\.63\\.2 ", "cored: site-b connected from 10.0.63.2 (behind rb)");
        writeSim(b, "core", RO_READER, "phone");
        b.mutate(r -> insertSim(r, "phone", "phone"), "right-click the phone's LTE Modem bay with the SIM");
        b.note("[scripted] The cart becomes a Sable ship (a vehicle a player would build); the phone rides it");
        b.mutate(CellularScenarios::assembleCart, "cart assembled");
        b.waitMs(1_000, "the cart settles");
        celldConnects(b, "phone", "the phone attaches (near site A)");
        b.expect("phone", "^\\s+cell\\s+cell-red cell 1, channel 2", "on site A (cell 1)");
        b.mutate(r -> run(r).notes.put("ip", cellAddress(r, "phone")), "note the phone's address");
        b.until("phone", "ping 10.0.64.10 -n 1", "^1 packets sent, 1 received", "the phone reaches the server through site A, ra and the core");
        b.send("phone", "ssh root@10.0.64.10");
        b.expect("phone", "logged in as root", "SSH session to the server over LTE");
        b.send("phone", "echo before-handover");
        b.expect("phone", "^before-handover\\s*$", "a command runs on the server");
        b.note("[scripted] The cart drives " + (int) RO_DRIVE + " blocks east, past the midpoint towards site B");
        b.await(CellularScenarios::drive, "the cart reached site B's side", 20_000);
        b.send("core", "cored handovers");
        b.expect("core", "handover " + IMSI + " cell 1 -> 2 \\(ip 100\\.65\\.13\\.\\d+ kept\\)", "cored handovers: the phone moved from cell 1 to"
                + " cell 2 and kept its address");
        b.send("phone", "echo after-handover");
        b.expect("phone", "^after-handover\\s*$", "the same SSH session works after the handover");
        b.send("phone", "exit");
        b.expect("phone", "Connection closed\\.", "logged out");
        // Retyped until it answers: a line typed while ssh is still exiting is lost.
        b.until("phone", "cell status", "^\\s+cell\\s+cell-red cell 2, channel 2", "cell: now on site B (cell 2)");
        b.expect("phone", "handovers [1-9]", "cell counts the handover");
        b.await(r -> {
            String before = run(r).notes.get("ip"), now = cellAddress(r, "phone");
            if (before == null || now == null) return "address before " + before + ", now " + now;
            if (!before.equals(now)) r.fail("the address changed from " + before + " to " + now);
            r.say("§7  the phone kept " + now + " across the handover");
            return null;
        }, "the phone's address is the one it had at site A", 1_000);
        b.send("core", "cored sessions");
        b.expect("core", "^session \\d+ " + IMSI + " 100\\.65\\.13\\.\\d+ active site-b 2", "cored: the session is at site-b now");
        b.timeLimit(60_000);
        return b.build();
    }

    static void assembleCart(ScenarioRun r) {
        if (!SableCompat.isLoaded()) {
            r.fail("Sable is not loaded: cell_roaming moves the cart as a Sable ship");
            return;
        }
        Run s = run(r);
        List<BlockPos> blocks = new ArrayList<>();
        for (BlockPos p : RO_CART) blocks.add(r.abs(p));
        blocks.add(r.abs(RO_PHONE));
        s.ship = SableShips.assemble(r.level(), r.abs(RO_PHONE), blocks, r.abs(new BlockPos(5, 3, 1)), r.abs(new BlockPos(7, 4, 3)));
        if (s.ship == null) {
            r.fail("cart assembly failed");
            return;
        }
        s.home = s.ship.position();
        SableShips.hold(s.ship, s.home.x, s.home.y, s.home.z, new double[] {0, 0, 0, 1});
        r.relocate("phone", s.ship.plotPos(r.abs(RO_PHONE)));
    }

    static String drive(ScenarioRun r) {
        Run s = RUNS.get(r);
        if (s == null || s.ship == null || s.ship.removed()) {
            r.fail("no cart");
            return "failed";
        }
        s.driven = Math.min(RO_DRIVE, s.driven + RO_SPEED);
        SableShips.hold(s.ship, s.home.x + s.driven, s.home.y, s.home.z, new double[] {0, 0, 0, 1});
        if (s.driven < RO_DRIVE) return String.format(Locale.ROOT, "driven %.1f of %.0f blocks", s.driven, RO_DRIVE);
        FramedModemModule m = modemOf(r, "phone");
        if (m != null && m.pose() != null)
            r.say(String.format(Locale.ROOT, "§7  cart at x+%.0f: %.1f m from A's antenna, %.1f m from B's", s.driven,
                    distance(m.pose(), r.abs(CellNetScenarios.panel(RO_A, 1))), distance(m.pose(), r.abs(CellNetScenarios.panel(RO_B, -1)))));
        return null;
    }

    /** Diagnosis appended to a failure: the screens of the run's computers. */
    public static String diagnoseRun(ScenarioRun r) {
        StringBuilder sb = new StringBuilder();
        for (String n : r.scenario().nodes.keySet()) {
            String s = r.screen(n);
            if (s == null || s.isBlank()) continue;
            String[] lines = s.stripTrailing().split("\n");
            sb.append("--- ").append(n).append(" (last lines)\n");
            for (int i = Math.max(0, lines.length - 12); i < lines.length; i++) sb.append(lines[i]).append('\n');
        }
        return sb.toString();
    }
}
//?}
