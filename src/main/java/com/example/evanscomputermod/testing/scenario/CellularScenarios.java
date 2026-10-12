package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.TerminalBlock;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.module.ModuleBays;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.antenna.RadioAntennaContent;
import com.example.evanscomputermod.radio.cellular.CellAntennaBlock;
import com.example.evanscomputermod.radio.cellular.CellPackets;
import com.example.evanscomputermod.radio.cellular.CellTech;
import com.example.evanscomputermod.radio.cellular.CellTowerBlockEntity;
import com.example.evanscomputermod.radio.cellular.CellularContent;
import com.example.evanscomputermod.radio.cellular.ModemModule;
import com.example.evanscomputermod.radio.cellular.SimCard;
import com.example.evanscomputermod.radio.cellular.SimCardItem;
import com.example.evanscomputermod.radio.cellular.TowerLink;
import com.example.evanscomputermod.radio.compat.sable.SableShips;
import com.example.evanscomputermod.radio.medium.PathTracer;
import com.example.evanscomputermod.radio.medium.WorldMediumContent;
import com.example.evanscomputermod.radio.medium.WorldRadioMedium;
import com.example.evanscomputermod.sable.SableCompat;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.phys.Vec3;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.WeakHashMap;

/**
 * Cellular scenarios (GUI roadmap P7, docs/gui/CELLULAR.md), spawnable with
 * {@code /ecm scenario spawn <name>} and run by the {@code ecm_cellular}
 * GameTests. Built and operated as a player would: the Cell Tower, its mast
 * and antennas, cables, computers and modems are placed and clicked in; the
 * tower is set up in its carrier screen (the same handler as the screen's
 * Apply button); SIMs are issued by right-clicking the tower with a blank SIM
 * card and clicked into the installed modem's bay; everything else is typed.
 * Only vanilla terrain (floors, the ridge) is set directly, and the cart of
 * {@code cell_handover} is moved as a Sable ship by script (a player would
 * have to build a vehicle).
 *
 * <ul>
 *   <li>{@code cell_lte_far}: a computer with an LTE modem gets a DHCP lease from,
 *       and pings, a server cabled to a cell tower 300 blocks away (control: no SIM,
 *       no carrier, no lease).</li>
 *   <li>{@code cell_mmwave_hill}: a mmWave tower; a computer in line of sight attaches
 *       and pings at 256QAM, one behind an earth ridge hears nothing (path-loss check);
 *       swapped to an LTE modem, that one gets through the ridge.</li>
 *   <li>{@code cell_handover}: two towers of one carrier on one cable segment; a computer
 *       on a moving cart keeps an SSH session open while it hands over from one tower to
 *       the other.</li>
 *   <li>{@code cell_wrong_carrier}: a SIM of carrier B hears carrier A's tower but never
 *       attaches (control: an A SIM attaches at once).</li>
 * </ul>
 */
public final class CellularScenarios {
    static final String PROMPT = PlayerKit.PROMPT;

    private CellularScenarios() {}

    /** One antenna on a mast: which technology, how high above the base station (1 = first mast block) and which side. */
    record Mount(CellTech tech, int height, Direction side) {}

    // ------------------------------------------------------------ kit

    /** Every block a tower at {@code base} with a mast of {@code mast} blocks and these antennas occupies. */
    static List<BlockPos> towerBlocks(BlockPos base, int mast, List<Mount> mounts) {
        List<BlockPos> out = new ArrayList<>();
        out.add(base);
        for (int i = 1; i <= mast; i++) out.add(base.above(i));
        for (Mount m : mounts) out.add(base.above(m.height()).relative(m.side()));
        return out;
    }

    /**
     * The player builds a Cell Tower: base station (standing on {@code standOn}), a Lattice Mast
     * column clicked on top, antennas clicked onto the mast's sides, then the carrier screen:
     * carrier and channels, Apply.
     */
    static void tower(ScenarioRun r, BlockPos rel, Direction standOn, int mast, List<Mount> mounts, String carrier, int lte, int mm) {
        BlockPos base = r.abs(rel);
        r.player().place(CellularContent.CELL_TOWER.get(), base, standOn);
        List<BlockPos> column = new ArrayList<>();
        for (int i = 1; i <= mast; i++) column.add(rel.above(i));
        PlayerKit.placeRun(r, RadioAntennaContent.LATTICE_MAST.get(), column, standOn);
        for (Mount m : mounts) {
            BlockPos at = base.above(m.height()).relative(m.side());
            r.player().placeAgainst(CellularContent.antenna(m.tech()), at, m.side(), m.side().getOpposite(),
                    s -> s.getValue(CellAntennaBlock.FACING) == m.side());
        }
        String reply = configure(r, rel, standOn, carrier, lte, mm);
        if (!reply.equals("Settings applied")) throw new IllegalStateException("tower screen at " + base + " said: " + reply);
        r.say("§7  Cell Tower at " + base.toShortString() + ": carrier " + carrier + ", LTE ch " + lte + ", mmWave ch " + mm + ", "
                + mounts.size() + " antenna(s) on a " + mast + "-block mast (" + reply + ")");
    }

    /** Open the tower's screen (right-click) and press Apply with these settings; returns the screen's reply. */
    static String configure(ScenarioRun r, BlockPos rel, Direction from, String carrier, int lte, int mm) {
        BlockPos p = r.abs(rel);
        r.player().useEmpty(p, from, false);
        r.player().entity().closeContainer();
        return CellPackets.act(r.player().entity(), CellPackets.Action.apply(p, carrier, lte, mm));
    }

    /** Right-click the tower with a blank SIM card: the carrier issues it; the player takes it out of their inventory. */
    static ItemStack issueSim(ScenarioRun r, BlockPos towerRel, Direction from) {
        List<String> said = r.player().use(new ItemStack(CellularContent.SIM_CARD.get()), r.abs(towerRel), from, false);
        var inv = r.player().entity().getInventory();
        for (int i = 0; i < inv.getContainerSize(); i++) {
            ItemStack s = inv.getItem(i);
            if (SimCardItem.isIssued(s)) {
                inv.setItem(i, ItemStack.EMPTY);
                return s;
            }
        }
        throw new IllegalStateException("the tower issued no SIM: the game said " + said);
    }

    /** The terminal's face with its left bay (where {@link ScenarioPlayer#installModule} puts modules). */
    static Direction leftFace(ScenarioRun r, String node) {
        Direction facing = r.level().getBlockState(r.where(node)).getValue(TerminalBlock.FACING);
        for (Direction d : Direction.Plane.HORIZONTAL) if (ModuleBays.bayOnFace(facing, d) == ModuleBays.LEFT) return d;
        throw new IllegalStateException("no left bay face");
    }

    /** The player clicks an expansion card and a modem into the computer's left side. */
    static void modem(ScenarioRun r, String node, CellTech tech) {
        List<String> said = r.player().installModule(r.where(node), CellularContent.modem(tech));
        TerminalBlockEntity t = r.terminal(node);
        boolean in = false;
        for (int i = 0; t != null && i < ModuleBays.SLOTS; i++) in |= t.getModuleBays().getStack(i).is(CellularContent.modem(tech));
        if (!in) throw new IllegalStateException(tech.displayName + " modem not installed in " + node + ": " + said);
        r.say("§7  " + tech.displayName + " Modem clicked into " + node + " (expansion card first, left side)");
    }

    /** Right-click the modem's bay (left side, upper slot) with the SIM card. */
    static void insertSim(ScenarioRun r, String node, ItemStack sim) {
        BlockPos p = r.where(node);
        Direction left = leftFace(r, node);
        List<String> said = r.player().use(sim, p, left, ScenarioPlayer.faceCentre(p, left).add(0, 0.25, 0), false);
        ModemModule m = modemOf(r, node);
        SimCard s = SimCardItem.sim(sim);
        if (m == null || m.ownSim() == null || !m.ownSim().equals(s))
            throw new IllegalStateException("SIM not taken by " + node + "'s modem: the game said " + said);
        r.say("§7  SIM " + s.carrier() + " " + s.imsiText() + " clicked into " + node + "'s modem (" + String.join(" ", said) + ")");
    }

    /** Sneak-right-click the modem's bay with an empty hand: the module comes out (into the player's inventory). */
    static void removeModem(ScenarioRun r, String node) {
        BlockPos p = r.where(node);
        Direction left = leftFace(r, node);
        r.player().use(ItemStack.EMPTY, p, left, ScenarioPlayer.faceCentre(p, left).add(0, 0.25, 0), true);
        if (modemOf(r, node) != null) throw new IllegalStateException("modem still in " + node);
        r.player().entity().getInventory().clearContent();
        r.say("§7  modem taken out of " + node + " (sneak-right-click its bay)");
    }

    static ModemModule modemOf(ScenarioRun r, String node) {
        TerminalBlockEntity t = r.terminal(node);
        if (t == null) return null;
        for (int i = 0; i < ModuleBays.SLOTS; i++) if (t.getModuleBays().getModule(i) instanceof ModemModule m) return m;
        return null;
    }

    static CellTowerBlockEntity towerAt(ScenarioRun r, BlockPos rel) {
        return r.level().getBlockEntity(r.abs(rel)) instanceof CellTowerBlockEntity t ? t : null;
    }

    /** Null once the tower's carrier screen lists {@code node}'s SIM as attached on {@code tech}. */
    static String screenShowsAttached(ScenarioRun r, BlockPos towerRel, String node, CellTech tech) {
        CellTowerBlockEntity t = towerAt(r, towerRel);
        ModemModule m = modemOf(r, node);
        if (t == null || m == null || m.currentSim() == null) return "no tower / modem / SIM";
        CellPackets.TowerView v = CellPackets.view(t, null);
        for (CellPackets.SimRow s : v.sims())
            if (s.imsi() == m.currentSim().imsi()) {
                if (!s.attached().equals(tech.displayName)) return "screen: " + node + "'s SIM attached via '" + s.attached() + "'";
                r.say(String.format(Locale.ROOT, "§7  tower screen: %s %s on %s, %.0f dBm uplink, CQI %d, %d B used", s.label(),
                        SimCard.formatImsi(s.imsi()), s.attached(), s.rssiDbm(), s.cqi(), s.usedBytes()));
                return null;
            }
        return "the tower screen doesn't list " + node + "'s SIM";
    }

    /** Distance from {@code tech}'s antenna on the tower to {@code node}'s modem, m (NaN if unknown). */
    static double antennaDistance(ScenarioRun r, BlockPos towerRel, String node, CellTech tech) {
        CellTowerBlockEntity t = towerAt(r, towerRel);
        ModemModule m = modemOf(r, node);
        if (t == null || m == null || t.link(tech) == null || t.link(tech).pose() == null || m.link().pose() == null) return Double.NaN;
        return t.link(tech).pose().distanceTo(m.link().pose());
    }

    static void logScreen(ScenarioRun r, String node) {
        EvansComputerMod.LOGGER.info("[cellular] {} screen:\n{}", node, r.screen(node));
    }

    static String towerDiagnosis(ScenarioRun r, BlockPos rel) {
        CellTowerBlockEntity t = towerAt(r, rel);
        if (t == null) return "no tower at " + r.abs(rel);
        StringBuilder sb = new StringBuilder(String.join("\n", t.statusLines()));
        for (CellTech tech : CellTech.values()) {
            TowerLink l = t.link(tech);
            if (l == null) continue;
            sb.append(String.format(Locale.ROOT, "%n  %s: beacons %d, dl %d, ul %d, attaches %d, rejects %d, to cable %d, last '%s'",
                    tech.displayName, l.beacons(), l.dlFrames(), l.ulFrames(), l.attaches(), l.rejects(), l.cableOut(), l.lastEvent()));
        }
        return sb.toString();
    }

    // ------------------------------------------------------------ cell_lte_far

    /** The far site: 300 blocks east of the phone. */
    public static final int FAR = 300;
    static final BlockPos FAR_PHONE = new BlockPos(0, 1, 0);
    static final BlockPos FAR_SERVER = new BlockPos(FAR, 1, 0), FAR_TOWER = new BlockPos(FAR, 1, 3);
    static final List<BlockPos> FAR_CABLE = List.of(new BlockPos(FAR, 0, 0), new BlockPos(FAR, 0, 1), new BlockPos(FAR, 0, 2),
            new BlockPos(FAR, 0, 3));
    static final List<Mount> FAR_ANTENNAS = List.of(new Mount(CellTech.LTE, 8, Direction.WEST), new Mount(CellTech.LTE, 8, Direction.NORTH),
            new Mount(CellTech.LTE, 8, Direction.SOUTH));
    static final String FAR_CARRIER = "cell-far";
    static final String FAR_POOL = "pool eth0 10.0.30.10 10.0.30.50 router 10.0.30.1 dns 1.1.1.1 lease 3600";

    static List<BlockPos> farSite() {
        List<BlockPos> all = new ArrayList<>(PlayerKit.box(FAR - 4, 0, -3, FAR + 4, 10, 6));
        return all;
    }

    /**
     * {@code cell_lte_far}: the phone (an LTE modem) and, 300 blocks east (kept loaded with
     * /forceload), a server (10.0.30.1, running dhcpd) cabled to a Cell Tower with three
     * LTE sector antennas 8 blocks up a lattice mast, carrier cell-far on LTE channel 2.
     */
    public static Scenario lteFar() {
        List<BlockPos> floor = PlayerKit.box(-3, 0, -2, 3, 0, 2);
        var b = Scenario.builder("cell_lte_far",
                        "a computer with an LTE modem attaches to a cell tower 300 blocks away and gets a DHCP lease from, and pings,"
                                + " a server on the tower's backhaul cable; without a SIM it has no carrier")
                .asPlayer()
                .realTime()
                .host("phone", FAR_PHONE, "-")
                .remoteHost("server", FAR_SERVER, "10.0.30.1/24")
                .decor(new Scenario.Decor() {
                    @Override public List<BlockPos> footprint() {
                        return floor;
                    }

                    @Override public void terrain(ScenarioRun r) {
                        BlockPos lo = r.abs(new BlockPos(FAR - 4, 0, -3)), hi = r.abs(new BlockPos(FAR + 4, 0, 6));
                        r.say("§7  " + String.join(" ", r.player().command(String.format(Locale.ROOT, "/forceload add %d %d %d %d",
                                lo.getX(), lo.getZ(), hi.getX(), hi.getZ()))));
                        for (BlockPos p : farSite()) r.level().setBlock(r.abs(p), Blocks.AIR.defaultBlockState(), 3);
                        for (BlockPos p : PlayerKit.box(FAR - 4, 0, -3, FAR + 4, 0, 6))
                            if (!FAR_CABLE.contains(p)) r.level().setBlock(r.abs(p), Blocks.SMOOTH_STONE.defaultBlockState(), 3);
                        PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState());
                    }

                    @Override public void build(ScenarioRun r) {
                        for (BlockPos p : FAR_CABLE) r.player().place(ModBlocks.NETWORK_CABLE.get(), r.abs(p), Direction.UP);
                        tower(r, FAR_TOWER, Direction.NORTH, 8, FAR_ANTENNAS, FAR_CARRIER, 2, 1);
                        modem(r, "phone", CellTech.LTE);
                    }

                    @Override public void clear(ScenarioRun r) {
                        for (BlockPos p : farSite()) r.level().setBlock(r.abs(p), Blocks.AIR.defaultBlockState(), 3);
                        BlockPos lo = r.abs(new BlockPos(FAR - 4, 0, -3)), hi = r.abs(new BlockPos(FAR + 4, 0, 6));
                        r.player().command(String.format(Locale.ROOT, "/forceload remove %d %d %d %d", lo.getX(), lo.getZ(), hi.getX(), hi.getZ()));
                    }
                })
                .note("Setup (done for you): 300 blocks east, a server computer cabled (bottom face) to a Cell Tower with three LTE"
                        + " sector antennas 8 blocks up a Lattice Mast, set up in its screen as carrier " + FAR_CARRIER
                        + ", LTE channel 2. Here, a computer with an LTE Modem and no SIM.")
                .send("server", "ifconfig eth0 10.0.30.1/24")
                .expect("server", "eth0: inet 10\\.0\\.30\\.1/24", "server has 10.0.30.1");
        PlayerKit.typeFile(b, "server", "/etc/dhcpd.conf", FAR_POOL);
        b.send("server", "dhcpd &")
                .expect("server", "serving eth0 10\\.0\\.30\\.10-10\\.0\\.30\\.50/24 as 10\\.0\\.30\\.1", "dhcpd serving the pool")
                .note("Control: the modem has no SIM, so wwan0 has no carrier and DHCP gets nothing")
                .send("phone", "cell")
                .expect("phone", "^\\s+state\\s+no_sim", "cell: no SIM", "^\\s+state\\s+connected")
                .send("phone", "dhclient -t 3 wwan0")
                .expect("phone", "^dhclient: no lease on wwan0", "no lease without a SIM (the kernel keeps trying)", "^bound to")
                .note("Walk to the tower, right-click it with a blank SIM card (the carrier issues it), walk back and right-click"
                        + " the modem's bay with the SIM")
                .mutate(r -> insertSim(r, "phone", issueSim(r, FAR_TOWER, Direction.NORTH)), "SIM issued at the far tower and clicked into the modem")
                .until("phone", "cell", "^\\s+state\\s+connected", "the modem attaches to " + FAR_CARRIER + " over LTE")
                .mutate(r -> {
                    double d = antennaDistance(r, FAR_TOWER, "phone", CellTech.LTE);
                    r.say(String.format(Locale.ROOT, "§7  tower antenna to modem: %.1f m", d));
                    if (!(d > 290)) r.fail("the tower is only " + d + " m away");
                    logScreen(r, "phone");
                }, "the tower's antenna is 300 m from the modem")
                .send("phone", "dhclient wwan0")
                .expect("phone", "^(bound to |\\s+lease\\s+)10\\.0\\.30\\.\\d+/24", "DHCP lease over LTE from the server's dhcpd", "^dhclient:")
                .until("phone", "ping 10.0.30.1 -n 1", "^1 packets sent, 1 received", "the phone reaches the server through the tower")
                .ping("phone", "10.0.30.1", 3, 3, "3 of 3 pings over 300 m of LTE")
                .await(r -> screenShowsAttached(r, FAR_TOWER, "phone", CellTech.LTE), "the tower's carrier screen lists the SIM, attached on LTE",
                        10_000)
                .timeLimit(60_000);
        return b.build();
    }

    // ------------------------------------------------------------ cell_mmwave_hill

    static final BlockPos HILL_SERVER = new BlockPos(0, 1, 0), HILL_TOWER = new BlockPos(0, 1, 3);
    static final BlockPos HILL_LOS = new BlockPos(-6, 1, 28), HILL_HIDDEN = new BlockPos(6, 1, 28);
    static final List<BlockPos> HILL_CABLE = List.of(new BlockPos(0, 0, 0), new BlockPos(0, 0, 1), new BlockPos(0, 0, 2), new BlockPos(0, 0, 3));
    static final List<Mount> HILL_ANTENNAS = List.of(new Mount(CellTech.MMWAVE, 5, Direction.SOUTH), new Mount(CellTech.LTE, 4, Direction.SOUTH));
    static final List<BlockPos> RIDGE = PlayerKit.box(3, 1, 23, 10, 6, 25);
    static final String HILL_CARRIER = "cell-hill";

    static final class HillState {
        ItemStack lteSim;
    }

    static final Map<ScenarioRun, HillState> HILL = Collections.synchronizedMap(new WeakHashMap<>());

    /**
     * {@code cell_mmwave_hill}: a server (10.0.31.1) cabled to a tower with a mmWave array
     * 5 blocks up and an LTE panel 4 up, both facing south (carrier cell-hill, mmWave ch 1,
     * LTE ch 4). 25 blocks south: "los" (mmWave modem) in the open and "hidden" (mmWave modem)
     * behind a 3-block-thick earth ridge taller than the line between the antennas.
     */
    public static Scenario mmwaveHill() {
        List<BlockPos> floor = new ArrayList<>(PlayerKit.box(-9, 0, -2, 11, 0, 30));
        floor.removeAll(HILL_CABLE);
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(HILL_CABLE);
        foot.addAll(RIDGE);
        foot.addAll(towerBlocks(HILL_TOWER, 5, HILL_ANTENNAS));
        var b = Scenario.builder("cell_mmwave_hill",
                        "mmWave works in line of sight and fails behind an earth ridge (path-loss check); the computer behind the ridge"
                                + " gets through on LTE")
                .asPlayer()
                .realTime()
                .host("server", HILL_SERVER, "10.0.31.1/24")
                .host("los", HILL_LOS, "-")
                .host("hidden", HILL_HIDDEN, "-")
                .decor(PlayerKit.decor(foot, r -> {
                    PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState());
                    PlayerKit.fill(r, RIDGE, Blocks.DIRT.defaultBlockState());
                    for (BlockPos p : PlayerKit.box(3, 6, 23, 10, 6, 25)) r.level().setBlock(r.abs(p), Blocks.GRASS_BLOCK.defaultBlockState(), 3);
                }, r -> {
                    for (BlockPos p : HILL_CABLE) r.player().place(ModBlocks.NETWORK_CABLE.get(), r.abs(p), Direction.UP);
                    tower(r, HILL_TOWER, Direction.NORTH, 5, HILL_ANTENNAS, HILL_CARRIER, 4, 1);
                    modem(r, "los", CellTech.MMWAVE);
                    modem(r, "hidden", CellTech.MMWAVE);
                }, r -> HILL.remove(r)))
                .note("Setup (done for you): a server cabled to a Cell Tower (carrier " + HILL_CARRIER + ") with a mmWave array"
                        + " and an LTE panel facing south; 25 blocks south, los (in the open) and hidden (behind an earth ridge), each"
                        + " with a mmWave Modem and a SIM the tower issued.")
                .mutate(r -> {
                    insertSim(r, "los", issueSim(r, HILL_TOWER, Direction.NORTH));
                    insertSim(r, "hidden", issueSim(r, HILL_TOWER, Direction.NORTH));
                    HILL.computeIfAbsent(r, k -> new HillState()).lteSim = issueSim(r, HILL_TOWER, Direction.NORTH);
                }, "three SIMs issued at the tower (right-click with blank SIM cards); one clicked into each modem")
                .send("server", "ifconfig eth0 10.0.31.1/24")
                .expect("server", "eth0: inet 10\\.0\\.31\\.1/24", "server has 10.0.31.1")
                .send("los", "ifconfig wwan0 10.0.31.2/24")
                .expect("los", "wwan0: inet 10\\.0\\.31\\.2/24", "los has 10.0.31.2 on wwan0")
                .send("hidden", "ifconfig wwan0 10.0.31.3/24")
                .expect("hidden", "wwan0: inet 10\\.0\\.31\\.3/24", "hidden has 10.0.31.3 on wwan0")
                .note("Line of sight: mmWave attaches at a 256QAM rate")
                .until("los", "cell", "^\\s+state\\s+connected", "los attaches over mmWave")
                .send("los", "cell")
                .expect("los", "^\\s+downlink\\s+CQI 1[6-8],", "a 256QAM downlink (CQI 16-18) in line of sight")
                .until("los", "ping 10.0.31.1 -n 1", "^1 packets sent, 1 received", "los reaches the server over mmWave")
                .ping("los", "10.0.31.1", 3, 3, "3 of 3 pings over mmWave")
                .note("Control: behind the ridge, mmWave hears nothing at all")
                .mutate(CellularScenarios::checkHillPaths, "path loss tower -> los and tower -> hidden at 28 GHz and 811 MHz")
                .send("hidden", "cell scan")
                .expect("hidden", "no towers heard", "the hidden computer hears no tower", "^cell-hill")
                .send("hidden", "cell")
                .expect("hidden", "^\\s+state\\s+(searching|attaching)", "not attached behind the ridge", "^\\s+state\\s+connected")
                .ping("hidden", "10.0.31.1", 1, 0, "no reply over mmWave behind the ridge")
                .note("Swap the hidden computer's modem for an LTE Modem (sneak-right-click the bay, click the LTE Modem in, then a SIM)")
                .mutate(r -> {
                    removeModem(r, "hidden");
                    modem(r, "hidden", CellTech.LTE);
                    insertSim(r, "hidden", HILL.get(r).lteSim);
                }, "hidden now has an LTE Modem with its own SIM")
                .until("hidden", "cell", "^\\s+state\\s+connected", "LTE attaches through the ridge")
                .until("hidden", "ifconfig wwan0 10.0.31.3/24", "wwan0: inet 10\\.0\\.31\\.3/24", "wwan0 (new modem, new MAC) configured again")
                .until("hidden", "ping 10.0.31.1 -n 1", "^1 packets sent, 1 received", "hidden reaches the server over LTE through the ridge")
                .await(r -> screenShowsAttached(r, HILL_TOWER, "hidden", CellTech.LTE), "tower screen: hidden's new SIM attached on LTE", 10_000)
                .timeLimit(60_000);
        return b.build();
    }

    /** The traced paths: los clear at 28 GHz, hidden blocked (> 100 dB of earth); LTE through the ridge costs tens of dB, not hundreds. */
    static void checkHillPaths(ScenarioRun r) {
        WorldRadioMedium m = WorldMediumContent.medium();
        var w = WorldMediumContent.rfWorld(r.level());
        CellTowerBlockEntity t = towerAt(r, HILL_TOWER);
        if (m == null || w == null || t == null || t.link(CellTech.MMWAVE) == null || t.link(CellTech.LTE) == null) {
            r.fail("no in-world medium or tower");
            return;
        }
        var c = new WorldRadioMedium.Conditions(r.level().getDayTime(), r.level().isThundering(), true);
        Pose mm = t.link(CellTech.MMWAVE).pose(), lte = t.link(CellTech.LTE).pose();
        Pose los = modemOf(r, "los").link().pose(), hid = modemOf(r, "hidden").link().pose();
        double f28 = CellTech.MMWAVE.downlink(1).centerHz(), f8 = CellTech.LTE.downlink(4).centerHz();
        PathTracer.Result a = m.traceNow(w, mm, los, f28, c), b = m.traceNow(w, mm, hid, f28, c), l = m.traceNow(w, lte, hid, f8, c);
        r.say(String.format(Locale.ROOT, "§7  28 GHz tower->los: %.1f m, %.1f dB (walls %.1f, diffraction %.1f, %s)", a.distanceM(), a.totalDb(),
                a.obstructionDb(), a.diffractionDb(), a.lineOfSight() ? "LOS" : "NLOS"));
        r.say(String.format(Locale.ROOT, "§7  28 GHz tower->hidden: %.1f m, %.1f dB (walls %.1f, diffraction %.1f, %s)", b.distanceM(), b.totalDb(),
                b.obstructionDb(), b.diffractionDb(), b.lineOfSight() ? "LOS" : "NLOS"));
        r.say(String.format(Locale.ROOT, "§7  811 MHz tower->hidden: %.1f m, %.1f dB (walls %.1f, diffraction %.1f)", l.distanceM(), l.totalDb(),
                l.obstructionDb(), l.diffractionDb()));
        if (!a.lineOfSight() || a.obstructionDb() > 3) r.fail("tower -> los is not a clear line of sight: " + a.obstructionDb() + " dB of walls");
        else if (b.totalDb() - a.totalDb() < 100) r.fail("the ridge only costs " + (b.totalDb() - a.totalDb()) + " dB at 28 GHz");
        else if (l.totalDb() - b.totalDb() > -50) r.fail("LTE through the ridge isn't far better than mmWave: " + l.totalDb() + " vs " + b.totalDb());
    }

    // ------------------------------------------------------------ cell_handover

    static final BlockPos HO_SERVER = new BlockPos(20, 1, 0), HO_A = new BlockPos(0, 1, 0), HO_B = new BlockPos(40, 1, 0);
    static final BlockPos HO_PHONE = new BlockPos(4, 2, 6);
    static final List<BlockPos> HO_CART = PlayerKit.box(3, 1, 5, 5, 1, 7);
    static final List<Mount> HO_A_ANTENNAS = List.of(new Mount(CellTech.LTE, 3, Direction.EAST), new Mount(CellTech.LTE, 3, Direction.SOUTH));
    static final List<Mount> HO_B_ANTENNAS = List.of(new Mount(CellTech.LTE, 3, Direction.WEST), new Mount(CellTech.LTE, 3, Direction.SOUTH));
    static final String HO_CARRIER = "cell-ho";
    /** How far east the cart drives, blocks; and how fast (blocks per tick). */
    static final double HO_DRIVE = 32, HO_SPEED = 0.25;

    static final class CartState {
        SableShips.Ship ship;
        Vec3 home;
        double driven;
    }

    static final Map<ScenarioRun, CartState> CARTS = Collections.synchronizedMap(new WeakHashMap<>());

    static List<BlockPos> hoCable() {
        List<BlockPos> c = new ArrayList<>();
        for (int x = 0; x <= 40; x++) c.add(new BlockPos(x, 0, 0));
        return c;
    }

    /**
     * {@code cell_handover}: towers A (x 0) and B (x 40) of carrier cell-ho on LTE channel 3,
     * each with two sector panels 3 blocks up (sector panels are tilted down with a narrow vertical beam: right under a tall mast a phone hears the far tower better), on one cable run under the floor that also
     * reaches the server (10.0.32.1, sshd) at x 20. The phone (10.0.32.2 on wwan0) rides a
     * plank cart near A; it SSHes into the server, the cart drives 32 blocks east past B's
     * side of the midpoint, the modem hands over to B, and the same SSH session still works.
     */
    public static Scenario handover() {
        List<BlockPos> cable = hoCable();
        List<BlockPos> floor = new ArrayList<>(PlayerKit.box(-2, 0, -2, 42, 0, 9));
        floor.removeAll(cable);
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(cable);
        foot.addAll(HO_CART);
        foot.addAll(towerBlocks(HO_A, 3, HO_A_ANTENNAS));
        foot.addAll(towerBlocks(HO_B, 3, HO_B_ANTENNAS));
        var b = Scenario.builder("cell_handover",
                        "a computer on a moving cart keeps an SSH session open while its LTE modem hands over between two towers of"
                                + " one carrier on one cable segment (the cart's movement is scripted)")
                .asPlayer()
                .realTime()
                .host("server", HO_SERVER, "10.0.32.1/24")
                .host("phone", HO_PHONE, "-")
                .decor(new Scenario.Decor() {
                    @Override public List<BlockPos> footprint() {
                        return foot;
                    }

                    @Override public void terrain(ScenarioRun r) {
                        PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState());
                        PlayerKit.fill(r, HO_CART, Blocks.SPRUCE_PLANKS.defaultBlockState());
                    }

                    @Override public void build(ScenarioRun r) {
                        PlayerKit.placeRun(r, ModBlocks.NETWORK_CABLE.get(), cable, Direction.NORTH);
                        tower(r, HO_A, Direction.NORTH, 3, HO_A_ANTENNAS, HO_CARRIER, 3, 1);
                        tower(r, HO_B, Direction.NORTH, 3, HO_B_ANTENNAS, HO_CARRIER, 3, 1);
                        modem(r, "phone", CellTech.LTE);
                    }

                    @Override public void clear(ScenarioRun r) {
                        CartState s = CARTS.remove(r);
                        if (s != null && s.ship != null && !s.ship.removed()) {
                            SableShips.release(s.ship);
                            SableShips.discard(s.ship);
                        }
                    }
                })
                .note("Setup (done for you): Cell Towers A and B (carrier " + HO_CARRIER + ", LTE channel 3) 40 blocks apart on one cable"
                        + " run with the server between them; the phone, with an LTE Modem and a SIM, on a plank cart near A.")
                .mutate(r -> insertSim(r, "phone", issueSim(r, HO_A, Direction.NORTH)), "SIM issued at tower A, clicked into the phone's modem")
                .send("server", "ifconfig eth0 10.0.32.1/24")
                .expect("server", "eth0: inet 10\\.0\\.32\\.1/24", "server has 10.0.32.1")
                .send("server", "sshd &")
                .expect("server", "sshd: starting on port 22", "sshd listening on the server")
                .send("phone", "ifconfig wwan0 10.0.32.2/24")
                .expect("phone", "wwan0: inet 10\\.0\\.32\\.2/24", "phone has 10.0.32.2 on wwan0")
                .until("phone", "cell", "^\\s+state\\s+connected", "the phone attaches (near tower A)")
                .note("[scripted] The cart becomes a Sable ship (a vehicle a player would build)")
                .mutate(CellularScenarios::assembleCart, "cart assembled")
                .await(CellularScenarios::cartReady, "the phone runs on the cart and is attached to tower A", 20_000)
                .until("phone", "ping 10.0.32.1 -n 1", "^1 packets sent, 1 received", "the phone reaches the server through tower A")
                .send("phone", "ssh root@10.0.32.1")
                .expect("phone", "logged in as root", "SSH session to the server over LTE")
                .send("phone", "echo before-handover")
                .expect("phone", "^before-handover\\s*$", "a command runs on the server")
                .note("[scripted] The cart drives " + (int) HO_DRIVE + " blocks east, past the midpoint towards tower B")
                .await(CellularScenarios::drive, "the cart reached tower B's side", 20_000)
                .await(CellularScenarios::handedOver, "tower B serves the phone and tower A let it go", 15_000)
                .send("phone", "echo after-handover")
                .expect("phone", "^after-handover\\s*$", "the same SSH session works after the handover")
                .send("phone", "exit")
                .expect("phone", "Connection closed\\.", "logged out")
                .send("phone", "cell")
                .expect("phone", "handovers [1-9]", "cell counts the handover")
                .mutate(r -> logScreen(r, "phone"), "log the phone's screen")
                .timeLimit(60_000);
        return b.build();
    }

    static void assembleCart(ScenarioRun r) {
        if (!SableCompat.isLoaded()) {
            r.fail("Sable is not loaded: cell_handover moves the cart as a Sable ship");
            return;
        }
        CartState s = CARTS.computeIfAbsent(r, k -> new CartState());
        List<BlockPos> blocks = new ArrayList<>();
        for (BlockPos p : HO_CART) blocks.add(r.abs(p));
        blocks.add(r.abs(HO_PHONE));
        s.ship = SableShips.assemble(r.level(), r.abs(HO_PHONE), blocks, r.abs(new BlockPos(3, 1, 5)), r.abs(new BlockPos(5, 2, 7)));
        if (s.ship == null) {
            r.fail("cart assembly failed");
            return;
        }
        s.home = s.ship.position();
        SableShips.hold(s.ship, s.home.x, s.home.y, s.home.z, new double[] {0, 0, 0, 1});
        r.relocate("phone", s.ship.plotPos(r.abs(HO_PHONE)));
    }

    static String cartReady(ScenarioRun r) {
        TerminalBlockEntity pc = r.terminal("phone");
        if (pc == null || pc.getComputer() == null) return "no running phone at " + r.where("phone");
        ModemModule m = modemOf(r, "phone");
        if (m == null || !m.link().linkUp()) return "modem " + (m == null ? "missing" : m.link().state() + ": " + m.link().message());
        CellTowerBlockEntity a = towerAt(r, HO_A);
        if (a == null || a.link(CellTech.LTE) == null || !a.link(CellTech.LTE).serves(m.currentSim().imsi()))
            return "tower A doesn't serve the phone: " + m.link().events();
        return null;
    }

    static String drive(ScenarioRun r) {
        CartState s = CARTS.get(r);
        if (s == null || s.ship == null || s.ship.removed()) {
            r.fail("no cart");
            return "failed";
        }
        s.driven = Math.min(HO_DRIVE, s.driven + HO_SPEED);
        SableShips.hold(s.ship, s.home.x + s.driven, s.home.y, s.home.z, new double[] {0, 0, 0, 1});
        if (s.driven < HO_DRIVE) return String.format(Locale.ROOT, "driven %.1f of %.0f blocks", s.driven, HO_DRIVE);
        r.say(String.format(Locale.ROOT, "§7  cart at x+%.0f: %.1f m from A's antennas, %.1f m from B's", s.driven,
                antennaDistance(r, HO_A, "phone", CellTech.LTE), antennaDistance(r, HO_B, "phone", CellTech.LTE)));
        return null;
    }

    static String handedOver(ScenarioRun r) {
        ModemModule m = modemOf(r, "phone");
        CellTowerBlockEntity a = towerAt(r, HO_A), b = towerAt(r, HO_B);
        if (m == null || a == null || b == null || m.currentSim() == null) return "missing modem/towers";
        long imsi = m.currentSim().imsi();
        boolean onB = b.link(CellTech.LTE).serves(imsi), onA = a.link(CellTech.LTE).serves(imsi);
        if (!onB || onA) return "A serves " + onA + ", B serves " + onB + "; modem " + m.link().state() + " " + m.link().events();
        if (m.link().handovers() < 1) return "B serves the phone but the modem counted no handover: " + m.link().events();
        r.say("§7  modem events: " + String.join("; ", m.link().events()));
        return null;
    }

    // ------------------------------------------------------------ cell_wrong_carrier

    static final BlockPos WC_SERVER = new BlockPos(0, 1, 0), WC_RED = new BlockPos(0, 1, 3), WC_BLUE = new BlockPos(6, 1, 3);
    static final BlockPos WC_PHONE = new BlockPos(0, 1, 12);
    static final List<BlockPos> WC_CABLE = List.of(new BlockPos(0, 0, 0), new BlockPos(0, 0, 1), new BlockPos(0, 0, 2), new BlockPos(0, 0, 3));
    static final List<Mount> WC_ANTENNAS = List.of(new Mount(CellTech.LTE, 4, Direction.SOUTH));

    /**
     * {@code cell_wrong_carrier}: carrier cell-red's tower (an LTE panel facing south, channel 5)
     * cabled to a server (10.0.33.1); carrier cell-blue's tower beside it has no antennas (it
     * only issues SIMs). The phone, 8 blocks south, has an LTE modem with a cell-blue SIM: it
     * hears the red tower but never attaches. A cell-red SIM attaches at once.
     */
    public static Scenario wrongCarrier() {
        List<BlockPos> floor = new ArrayList<>(PlayerKit.box(-3, 0, -2, 8, 0, 14));
        floor.removeAll(WC_CABLE);
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(WC_CABLE);
        foot.addAll(towerBlocks(WC_RED, 4, WC_ANTENNAS));
        foot.add(WC_BLUE);
        var b = Scenario.builder("cell_wrong_carrier",
                        "a SIM from carrier cell-blue hears carrier cell-red's tower but never attaches to it; a cell-red SIM attaches")
                .asPlayer()
                .realTime()
                .host("server", WC_SERVER, "10.0.33.1/24")
                .host("phone", WC_PHONE, "-")
                .decor(PlayerKit.decor(foot, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()), r -> {
                    for (BlockPos p : WC_CABLE) r.player().place(ModBlocks.NETWORK_CABLE.get(), r.abs(p), Direction.UP);
                    tower(r, WC_RED, Direction.NORTH, 4, WC_ANTENNAS, "cell-red", 5, 2);
                    tower(r, WC_BLUE, Direction.NORTH, 0, List.of(), "cell-blue", 6, 3);
                    modem(r, "phone", CellTech.LTE);
                }, null))
                .note("Setup (done for you): carrier cell-red's Cell Tower (LTE, channel 5) cabled to the server; carrier cell-blue's"
                        + " tower beside it (no antennas) issued the phone's SIM.")
                .mutate(r -> insertSim(r, "phone", issueSim(r, WC_BLUE, Direction.NORTH)), "cell-blue SIM issued at the blue tower, clicked into the phone's modem")
                .send("server", "ifconfig eth0 10.0.33.1/24")
                .expect("server", "eth0: inet 10\\.0\\.33\\.1/24", "server has 10.0.33.1")
                .send("phone", "ifconfig wwan0 10.0.33.2/24")
                .expect("phone", "wwan0: inet 10\\.0\\.33\\.2/24", "phone has 10.0.33.2 on wwan0")
                .note("Control: the blue SIM hears the red tower but doesn't attach")
                .until("phone", "cell scan", "^cell-red\\s+LTE\\s+5\\s", "cell scan lists cell-red's tower on LTE channel 5")
                .waitMs(2_000, "time for the modem to have tried, if it were going to")
                .send("phone", "cell")
                .expect("phone", "^\\s+state\\s+searching - Searching for cell-blue", "still searching for cell-blue", "^\\s+state\\s+connected")
                .ping("phone", "10.0.33.1", 1, 0, "no reply: not attached")
                .await(CellularScenarios::redTowerUntouched, "the red tower got no attach request and lists no device", 5_000)
                .note("Right-click the red tower with a blank SIM card, then the modem's bay with the new SIM (the blue one pops out)")
                .mutate(r -> insertSim(r, "phone", issueSim(r, WC_RED, Direction.NORTH)), "cell-red SIM clicked into the modem")
                .until("phone", "cell", "^\\s+state\\s+connected", "the cell-red SIM attaches")
                .until("phone", "ping 10.0.33.1 -n 1", "^1 packets sent, 1 received", "the phone reaches the server through the red tower")
                .timeLimit(60_000);
        return b.build();
    }

    static String redTowerUntouched(ScenarioRun r) {
        CellTowerBlockEntity t = towerAt(r, WC_RED);
        if (t == null || t.link(CellTech.LTE) == null) return "no red tower";
        TowerLink l = t.link(CellTech.LTE);
        if (!l.onAir() || l.beacons() == 0) return "red tower not on the air: " + towerDiagnosis(r, WC_RED);
        if (l.attaches() + l.rejects() != 0 || l.deviceCount() != 0) {
            r.fail("control: the red tower saw " + l.attaches() + " attaches, " + l.rejects() + " rejects from a blue SIM");
            return "failed";
        }
        return null;
    }

    /** Diagnosis appended to a failure: every tower of the run. */
    public static String diagnoseRun(ScenarioRun r) {
        StringBuilder sb = new StringBuilder();
        for (BlockPos p : List.of(FAR_TOWER, HILL_TOWER, HO_A, HO_B, WC_RED))
            if (towerAt(r, p) != null) sb.append(towerDiagnosis(r, p)).append('\n');
        return sb.toString();
    }

}
//?}
