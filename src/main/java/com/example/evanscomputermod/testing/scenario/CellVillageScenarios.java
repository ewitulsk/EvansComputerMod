package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.CellPlan;
import com.example.evanscomputermod.computer.WorldNetwork;
import com.example.evanscomputermod.phone.PhoneContent;
import com.example.evanscomputermod.phone.PhoneHost;
import com.example.evanscomputermod.phone.PhoneItemData;
import com.example.evanscomputermod.phone.PhoneServer;
import com.example.evanscomputermod.radio.cellular.CellularContent;
import com.example.evanscomputermod.radio.cellular.SimCardItem;
import com.example.evanscomputermod.radio.framed.PhyProfile;
import com.example.evanscomputermod.testing.TechCellChecks;
import com.example.evanscomputermod.worldgen.TechVillageLocator;

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.server.level.TicketType;
import net.minecraft.world.entity.decoration.ItemFrame;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;

import java.nio.file.Files;
import java.util.*;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Cellular v2 phase 5 in a generated world: the Tech Village carrier network as a player meets
 * it (docs/gui/CELLULAR_V2.md "As built: phase 5"). {@code /ecm scenario spawn cell_tech_village}
 * near a Tech Village runs it from that village ({@code a}) to the next one ({@code b}); the
 * {@code tech} client suite runs the same definition in a fresh, naturally generated world.
 *
 * <ol>
 *   <li>In village a's Phone Store the player hits an item frame on the display table: the ePhone
 *       drops (LTE Modem and SIM fitted), the player picks it up and uses it (its first use gives
 *       it its computer). It boots, celld attaches to the store's cell site: the status bar shows
 *       the carrier, {@code cell} says connected on cell a.</li>
 *   <li>Over LTE the phone fetches the village's web page from the Data Center
 *       ({@code curl http://100.(64+a).0.10/}).</li>
 *   <li>The App Store: the Tech Village repository is already there (Repos lists it with its key
 *       pinned; nobody added it) with Discord, "Verified"; Discord is not installed. Install over
 *       LTE (the chat village's package server logs the download), Open; Discord looks on the
 *       network (the player allows it the network): the core relays the search to its ISP
 *       router, which answers with the ring's Discord backend, and Discord connects by itself.
 *       The player opens #general in the Tech Village server, sets a nickname and sends a
 *       message: the backend's log shows it.</li>
 *   <li>The player travels along the fiber to village b's Phone Store (12 blocks a second, beside
 *       the line, at head height over the ground): the phone is handed over from site to site
 *       ({@code handover cell x -> y} in celld's log) and keeps its address all the way.
 *       <b>[exception]</b> the holder is moved by script (a player riding along), in real time;
 *       the cell sites within 10 chunks of the phone are kept ticking, as a default server's
 *       simulation distance does for a player (the client-check server runs 3 to stay fast).</li>
 *   <li>At village b the player opens the Phone Store's chest, takes a SIM, fits it with an LTE
 *       Modem into a new ePhone (its slot screen): that phone attaches too.</li>
 * </ol>
 */
public final class CellVillageScenarios {
    private CellVillageScenarios() {}

    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();
    /** Travel speed along the route, blocks per second of real time (a fast horse). */
    static final double SPEED = 12;
    /** The ticking area around the phone (chunks): a default server's simulation distance. */
    static final int TICK_RADIUS = 10;
    private static final TicketType<ChunkPos> RIDE = TicketType.create("ecm_phone_ride", Comparator.comparingLong(ChunkPos::toLong));

    /** Per run. */
    static final class State {
        int a, b;
        TechVillageLocator.Visit va, vb;
        List<Vec3> route;
        int leg;
        double along;
        ChunkPos ticket;
        List<ChunkPos> sites;
        final Map<ChunkPos, Integer> ticking = new HashMap<>();
        long lastMs, lastReport;
        String ip, imsi;
        long rideTicks, servedTicks, startMs;
        int handoversAtStart;
        String report = "";
    }

    private static final Map<ScenarioRun, State> STATES = Collections.synchronizedMap(new WeakHashMap<>());

    static State st(ScenarioRun r) {
        return STATES.computeIfAbsent(r, k -> {
            State s = new State();
            var d = WorldNetwork.get(r.level());
            d.plan(r.level());
            s.a = d.nearest(r.origin()).number();
            s.b = s.a == 10 ? 1 : s.a + 1;
            s.va = TechVillageLocator.visit(r.level(), s.a);
            s.vb = TechVillageLocator.visit(r.level(), s.b);
            return s;
        });
    }

    static CellPlan plan(ScenarioRun r) {
        return WorldNetwork.get(r.level()).cells(r.level());
    }

    // ------------------------------------------------------------ taking a phone from the display

    /** Hit the first display frame that still holds a phone (survival: the phone drops), then pick it up. */
    static String takeDisplayPhone(ScenarioRun r) {
        State s = st(r);
        var p = r.player().entity();
        for (int i = 0; i < 36; i++) if (p.getInventory().getItem(i).is(PhoneContent.EPHONE.get())) return null;
        BlockPos table = TechCellChecks.store(s.va, TechCellChecks.TABLE);
        follow(r, s, Vec3.atCenterOf(table));   // the player's ticking area is where the player is
        var frames = r.level().getEntitiesOfClass(ItemFrame.class, new AABB(table).inflate(5, 3, 5));
        ItemFrame frame = frames.stream().filter(f -> f.getItem().is(PhoneContent.EPHONE.get()))
                .min(Comparator.comparingDouble(f -> f.distanceToSqr(Vec3.atCenterOf(table)))).orElse(null);
        boolean dropped = !r.level().getEntitiesOfClass(net.minecraft.world.entity.item.ItemEntity.class, new AABB(table).inflate(5, 3, 5),
                e -> e.getItem().is(PhoneContent.EPHONE.get())).isEmpty();
        if (frame != null && !dropped) {   // one hit: then wait for the dropped phone's pickup delay
            Vec3 eye = frame.position().add(0, 1.5, 0).add(Vec3.atLowerCornerOf(s.va.datacenter().getRotation().rotate(Direction.SOUTH).getNormal()).scale(1.5));
            r.player().lookFrom(eye, frame.position());
            p.setGameMode(GameType.SURVIVAL);
            try {
                p.attack(frame);
            } finally {
                p.setGameMode(GameType.CREATIVE);
            }
            if (!frame.getItem().isEmpty()) return "hitting the frame left the phone in it";
            r.say("§7  hit the display frame at " + frame.blockPosition().toShortString() + ": the phone drops");
        }
        return r.player().pickUp(table.above(), PhoneContent.EPHONE.get());
    }

    /** The phone in the main hand (hotbar slot 1) and used once: its first use gives it its computer. */
    static void holdAndUse(ScenarioRun r) {
        var p = PhoneScenarios.holder(r);
        Inventory inv = p.getInventory();
        int at = -1;
        // The display phone: it carries the computer generation provisioned for it (store.phone<slot>, the repository in /etc/pkg/repos).
        Set<UUID> display = new HashSet<>();
        var d = WorldNetwork.get(r.level());
        for (int slot = 1; slot <= CellPlan.PHONES; slot++) display.add(d.identity(r.level(), st(r).a, "store.phone" + slot));
        for (int i = 0; i < 36; i++) if (inv.getItem(i).is(PhoneContent.EPHONE.get()) && display.contains(PhoneItemData.id(inv.getItem(i)))) at = i;
        if (at < 0) {
            r.fail("no display phone of village " + st(r).a + "'s Phone Store in the inventory");
            return;
        }
        ItemStack phone = inv.getItem(at);
        inv.setItem(at, inv.getItem(0));
        inv.setItem(0, phone);
        inv.selected = 0;
        PhoneServer.track(p);
        r.player().useHeld(false);
        UUID id = PhoneItemData.id(inv.getItem(0));
        if (id == null || !display.contains(id)) {
            r.fail("using the phone did not keep its provisioned computer: " + id);
            return;
        }
        PhoneScenarios.st(r).phone = id;
        r.say("§7  holding the phone (computer " + id.toString().substring(0, 8) + "), SIM " + SimCardItem.identity(
                com.example.evanscomputermod.phone.PhoneHardware.of(inv.getItem(0)).sim()));
    }

    static String phoneScreen(ScenarioRun r, String re) {
        Matcher m = Pattern.compile(re, Pattern.MULTILINE).matcher(PhoneScenarios.screen(r));
        String last = null;
        while (m.find()) last = m.group(1);
        return last;
    }

    /** {@link #phoneScreen} on the screen with its wrapped lines joined (the phone's Terminal is 30 columns wide). */
    static String joined(ScenarioRun r, String re) {
        Matcher m = Pattern.compile(re).matcher(PhoneScenarios.screen(r).replace("\n", ""));
        String last = null;
        while (m.find()) last = m.group(1);
        return last;
    }

    static String celldLog(ScenarioRun r) {
        PhoneHost h = PhoneScenarios.host(r);
        try {
            return h == null || h.instance() == null ? "" : Files.readString(h.instance().storagePath().resolve("var/log/celld.log"));
        } catch (Exception e) {
            return "";
        }
    }

    static int count(String text, String re) {
        Matcher m = Pattern.compile(re, Pattern.MULTILINE).matcher(text);
        int n = 0;
        while (m.find()) n++;
        return n;
    }

    // ------------------------------------------------------------ the ride along the fiber

    static Vec3 ground(ScenarioRun r, double x, double z) {
        // The heightmap of a chunk that isn't loaded is the bottom of the world: load it first.
        r.level().getChunk(((int) Math.floor(x)) >> 4, ((int) Math.floor(z)) >> 4);
        int y = r.level().getHeight(Heightmap.Types.MOTION_BLOCKING_NO_LEAVES, (int) Math.floor(x), (int) Math.floor(z));
        return new Vec3(x, y + 1.62, z);
    }

    /** Store a's door, along the chord a few blocks beside the fiber, to store b's door: eye positions. */
    static List<Vec3> route(ScenarioRun r) {
        State s = st(r);
        var ring = WorldNetwork.get(r.level()).ring(r.level());
        int[][] path = ring.path(WorldNetwork.chord(s.a, s.b));
        var start = WorldNetwork.get(r.level()).villages.get(s.a - 1).endpoint(true);
        if (path[0][0] != start.getX() || path[0][2] != start.getZ()) {
            int[][] rev = new int[path.length][];
            for (int i = 0; i < path.length; i++) rev[i] = path[path.length - 1 - i];
            path = rev;
        }
        List<Vec3> out = new ArrayList<>();
        BlockPos doorA = TechCellChecks.store(s.va, TechCellChecks.STORE_DOOR), doorB = TechCellChecks.store(s.vb, TechCellChecks.STORE_DOOR);
        Direction frontA = s.va.datacenter().getRotation().rotate(Direction.SOUTH), frontB = s.vb.datacenter().getRotation().rotate(Direction.SOUTH);
        out.add(Vec3.atCenterOf(TechCellChecks.store(s.va, TechCellChecks.STORE_INSIDE)).add(0, 1.12, 0));
        BlockPos outA = doorA.relative(frontA, 4);
        out.add(ground(r, outA.getX() + 0.5, outA.getZ() + 0.5));
        int[] first = path[Math.min(path.length - 1, 60)], last = path[Math.max(0, path.length - 61)];
        double dx = last[0] - first[0], dz = last[2] - first[2], len = Math.max(1, Math.hypot(dx, dz));
        double sx = -dz / len * 4, sz = dx / len * 4;   // four blocks to the side of the line
        for (int i = 60; i < path.length - 60; i += 16) out.add(ground(r, path[i][0] + 0.5 + sx, path[i][2] + 0.5 + sz));
        BlockPos outB = doorB.relative(frontB, 4);
        out.add(ground(r, outB.getX() + 0.5, outB.getZ() + 0.5));
        out.add(Vec3.atCenterOf(TechCellChecks.store(s.vb, TechCellChecks.STORE_INSIDE)).add(0, 1.12, 0));
        return out;
    }

    /**
     * Keep ticking what a player standing at {@code eye} ticks on a default server (10 chunks of
     * simulation distance) that matters here: the player's own chunk (entities: the dropped
     * phone) and every cell site of the two villages and their chord within 10 chunks. The rest
     * of that area is left alone, so the test server needn't generate a 21-chunk-wide band.
     */
    static void follow(ScenarioRun r, State s, Vec3 eye) {
        ChunkPos here = new ChunkPos(BlockPos.containing(eye));
        if (here.equals(s.ticket)) return;
        s.ticket = here;
        if (s.sites == null) {
            s.sites = new ArrayList<>();
            var plan = plan(r);
            s.sites.add(new ChunkPos(TechCellChecks.store(s.va, TechCellChecks.STORE_SITE)));
            s.sites.add(new ChunkPos(TechCellChecks.store(s.vb, TechCellChecks.STORE_SITE)));
            for (var sp : plan.spans(WorldNetwork.chord(s.a, s.b))) s.sites.add(new ChunkPos(sp.x() >> 4, sp.z() >> 4));
        }
        Map<ChunkPos, Integer> want = new HashMap<>();
        want.put(here, 2);   // entity ticking where the player is
        for (ChunkPos c : s.sites)
            if (Math.max(Math.abs(c.x - here.x), Math.abs(c.z - here.z)) <= TICK_RADIUS) want.putIfAbsent(c, 1);   // block ticking
        var src = r.level().getChunkSource();
        for (var e : want.entrySet())
            if (!e.getValue().equals(s.ticking.get(e.getKey()))) src.addRegionTicket(RIDE, e.getKey(), e.getValue(), e.getKey());
        for (var e : s.ticking.entrySet())
            if (!e.getValue().equals(want.get(e.getKey()))) src.removeRegionTicket(RIDE, e.getKey(), e.getValue(), e.getKey());
        s.ticking.clear();
        s.ticking.putAll(want);
    }

    static void release(ScenarioRun r) {
        State s = st(r);
        var src = r.level().getChunkSource();
        for (var e : s.ticking.entrySet()) src.removeRegionTicket(RIDE, e.getKey(), e.getValue(), e.getKey());
        s.ticking.clear();
        s.ticket = null;
    }

    /** The two nearest span sites: distance, ticking, the SDR's mode, the computer's last screen line (a check, for the log). */
    static String nearestSites(ScenarioRun r, State s, Vec3 eye) {
        var level = r.level();
        StringBuilder out = new StringBuilder();
        plan(r).spans(WorldNetwork.chord(s.a, s.b)).stream()
                .sorted(Comparator.comparingDouble(sp -> Math.hypot(sp.x() - eye.x, sp.z() - eye.z))).limit(2).forEach(sp -> {
                    var l = com.example.evanscomputermod.worldgen.CellSites.locate(level, sp);
                    if (l == null) return;
                    String mode = level.getBlockEntity(l.sdr()) instanceof com.example.evanscomputermod.radio.sdr.SdrBlockEntity sdr
                            ? sdr.getPeripheral().mode().toString() : "no SDR";
                    String last = "";
                    if (level.getBlockEntity(l.computer()) instanceof com.example.evanscomputermod.block.TerminalBlockEntity be && be.getDisplay() != null) {
                        String[] lines = ScenarioRun.screen(be.getDisplay()).strip().split("\n");
                        last = lines[lines.length - 1].strip();
                    }
                    out.append(sp.id()).append(" ").append((int) Math.hypot(sp.x() - eye.x, sp.z() - eye.z)).append(" m ")
                            .append(level.shouldTickBlocksAt(l.sdr()) ? "ticking" : "NOT ticking").append(" ").append(mode)
                            .append(" '").append(last).append("'; ");
                });
        return out.toString();
    }

    /** One tick of the ride: move SPEED along the route; null once at store b. Counts the ticks served by a cell. */
    static String ride(ScenarioRun r) {
        State s = st(r);
        if (s.route == null) {
            s.route = route(r);
            s.leg = 0;
            s.along = 0;
            s.startMs = System.currentTimeMillis();
            s.lastMs = s.startMs;
            s.handoversAtStart = count(celldLog(r), "^.*handover cell ");
            r.say("§7  riding " + s.route.size() + " waypoints from village " + s.a + "'s Phone Store to village " + s.b + "'s");
        }
        long now = System.currentTimeMillis();
        double left = Math.min(30, SPEED * (now - s.lastMs) / 1000.0);   // radios run in real time: so does the ride
        s.lastMs = now;
        while (left > 0 && s.leg < s.route.size() - 1) {
            Vec3 from = s.route.get(s.leg), to = s.route.get(s.leg + 1);
            double seg = from.distanceTo(to);
            if (s.along + left >= seg) {
                left -= seg - s.along;
                s.leg++;
                s.along = 0;
            } else {
                s.along += left;
                left = 0;
            }
        }
        Vec3 from = s.route.get(s.leg), to = s.route.get(Math.min(s.leg + 1, s.route.size() - 1));
        double seg = Math.max(1e-6, from.distanceTo(to));
        Vec3 eye = from.lerp(to, Math.min(1, s.along / seg));
        r.player().lookFrom(eye, to.equals(from) ? eye.add(1, 0, 0) : to);
        follow(r, s, eye);
        s.rideTicks++;
        if (CellPhoneScenarios.statusHas(r, "\ncarrier " + plan(r).carrier + "\n") == null) s.servedTicks++;
        if (now - s.lastReport >= 10_000) {
            s.lastReport = now;
            r.say("§7  " + (int) eye.x + " " + (int) eye.z + ": " + s.servedTicks * 100 / s.rideTicks + " % on a cell, "
                    + (count(celldLog(r), "^.*handover cell ") - s.handoversAtStart) + " handovers; " + nearestSites(r, s, eye));
        }
        if (s.leg >= s.route.size() - 1) {
            String log = celldLog(r);
            int handovers = count(log, "^.*handover cell ") - s.handoversAtStart;
            s.report = "ride " + (System.currentTimeMillis() - s.startMs) / 1000 + " s, " + s.rideTicks + " ticks, on a cell "
                    + s.servedTicks * 100 / Math.max(1, s.rideTicks) + " %, " + handovers + " handovers, " + count(log, "^.*attached cell ")
                    + " attaches in all";
            r.say("§7  arrived at village " + s.b + "'s Phone Store: " + s.report);
            return null;
        }
        return "riding (" + s.leg + "/" + s.route.size() + ")";
    }

    // ------------------------------------------------------------ the chest SIM

    /** Open village b's SIM chest, shift-click its first SIM into the inventory, close it. */
    static String takeChestSim(ScenarioRun r) {
        State s = st(r);
        BlockPos chest = TechCellChecks.store(s.vb, TechCellChecks.CHEST);
        Direction front = s.vb.datacenter().getRotation().rotate(Direction.SOUTH);
        r.player().lookFrom(Vec3.atCenterOf(chest.relative(front)).add(0, 0.6, 0), Vec3.atCenterOf(chest));
        var menu = r.player().openMenu(chest, front, false);
        if (menu == null) return "the chest did not open";
        ItemStack took = ItemStack.EMPTY;
        for (int i = 0; i < 27 && took.isEmpty(); i++)
            if (SimCardItem.isIssued(menu.getSlot(i).getItem())) took = r.player().shiftClick(i);
        r.player().closeScreen();
        if (took.isEmpty()) return "no SIM came out of the chest";
        s.imsi = SimCardItem.identity(took).imsiText();
        r.say("§7  took SIM " + s.imsi + " from village " + s.b + "'s Phone Store chest");
        return null;
    }

    /** A new ePhone (as crafted) in hotbar slot 1, used once (its computer), and an LTE Modem in the inventory. */
    static void secondPhone(ScenarioRun r) {
        var p = PhoneScenarios.holder(r);
        ItemStack phone = new ItemStack(PhoneContent.EPHONE.get());
        r.player().useInAir(phone, false);
        UUID id = PhoneItemData.id(phone);
        if (id == null) {
            r.fail("using the new phone gave it no computer");
            return;
        }
        p.getInventory().setItem(0, phone);
        p.getInventory().selected = 0;
        p.getInventory().setItem(21, new ItemStack(CellularContent.modem(PhyProfile.LTE)));
        PhoneScenarios.st(r).phone = id;
        r.say("\u00a77  a new ePhone (computer " + id.toString().substring(0, 8) + ") in hand and an LTE Modem in the inventory");
    }

    // ------------------------------------------------------------ the App Store and Discord

    static final String CHAT_NICK = "shopper", CHAT_HELLO = "hello from the phone store";

    /** The screen of an infrastructure computer: its block's display when it is on one, else the headless one. */
    static String infraScreen(ScenarioRun r, int village, String role) {
        var d = WorldNetwork.get(r.level());
        var host = com.example.evanscomputermod.computer.ComputerHost.get(r.level().getServer(), d.identity(r.level(), village, role));
        if (host.attachment() instanceof com.example.evanscomputermod.block.TerminalBlockEntity be && be.getDisplay() != null)
            return ScenarioRun.screen(be.getDisplay());
        return ScenarioRun.screen(host.headlessDisplay());
    }

    /** Null if the App Store's window shows every one of {@code texts} (exact texts). */
    static String storeShows(ScenarioRun r, String... texts) {
        var c = PhoneGuiScenarios.comp(r);
        if (c == null) return "no phone GUI";
        for (var w : c.windows()) {
            if (!w.appId().equals(CellPhoneScenarios.STORE) || !w.mapped()) continue;
            var shown = c.textsOf(w.id());
            for (String t : texts) if (!shown.contains(t)) return "the App Store shows " + shown;
            return null;
        }
        return "no App Store window";
    }

    /** Discord as the phone has it: null if not installed (app manager and /apps), else where it was found. */
    static String chatInstalled(ScenarioRun r) {
        var c = PhoneGuiScenarios.computer(r);
        if (c == null) return "no phone computer";
        if (c.getAppManager().manifest(CellPhoneScenarios.CHAT) != null) return "the app manager knows Discord";
        if (Files.exists(c.storagePath().resolve("apps/" + CellPhoneScenarios.CHAT))) return "/apps/" + CellPhoneScenarios.CHAT + " exists";
        return null;
    }

    /**
     * The display phone's App Store: the Tech Village repository came with the phone (its
     * /etc/pkg/repos, key pinned), so Discord is listed as Verified; install it over LTE, open it:
     * it finds the ring's Discord backend on the network by itself, and the player says something in
     * the Tech Village server's #general (docs/TECH_VILLAGE_NETWORK.md "Apps").
     */
    static void appStoreChat(Scenario.Builder b) {
        b.note("The App Store: the Tech Village repository is already listed with its key pinned (the phone came with it), with Discord"
                + " from a verified publisher; Discord is not installed yet. Install it over LTE, open it, allow the network: it finds the"
                + " ring's Discord backend by itself; say hello in #general");
        b.await(r -> {
            String why = chatInstalled(r);
            return why == null ? null : "control failed: " + why;
        }, "control: Discord is not installed on the phone", 2_000);
        PhoneGuiScenarios.openApp(b, "App Store", CellPhoneScenarios.STORE);
        b.await(r -> storeShows(r, "Discord", "Install", "1 apps from 1 repository"),
                "the App Store lists Discord from the Tech Village repository, not installed (its index over LTE)", 45_000);
        b.await(r -> CellPhoneScenarios.tapIn(r, "App Store", "Repos"), "tap Repos", 5_000);
        b.await(r -> {
            var repo = WorldNetwork.get(r.level()).repo(r.level());
            String why = storeShows(r, com.example.evanscomputermod.computer.VillageRepo.NAME + " (pinned key)", repo.url());
            if (why == null) r.say("\u00a77  Repositories: " + com.example.evanscomputermod.computer.VillageRepo.NAME + " " + repo.url() + " (pinned key)");
            return why;
        }, "Repositories: Tech-Village-Apps with its pinned key, nobody added it", 5_000);
        b.await(r -> CellPhoneScenarios.tapIn(r, "App Store", "< Back"), "tap Back", 5_000);
        b.await(r -> CellPhoneScenarios.tapIn(r, "App Store", "Discord"), "tap Discord: its page", 5_000);
        b.await(r -> storeShows(r, "Verified publisher", "Install"), "Discord's page: Verified publisher, Install", 5_000);
        b.await(r -> CellPhoneScenarios.tapIn(r, "App Store", "Install"), "tap Install", 5_000);
        b.await(r -> {
            int k = WorldNetwork.get(r.level()).repo(r.level()).village();
            String scr = infraScreen(r, k, WorldNetwork.REPO);
            // The package itself, not its icon (GET /dev.ecm.discord-1.0.0.ecmpkg/icon.png, which the list fetched).
            return Pattern.compile("(?m)GET /dev\\.ecm\\.discord-1\\.0\\.0\\.ecmpkg\\s*$").matcher(scr).find() ? null
                    : "package server (village " + k + "):\n" + scr;
        }, "the chat village's package server logs the download", 30_000);
        b.await(r -> {
            var c = PhoneGuiScenarios.computer(r);
            if (c == null || c.getAppManager().manifest(CellPhoneScenarios.CHAT) == null) return "not installed yet";
            try {
                String origin = Files.readString(c.storagePath().resolve("apps/" + CellPhoneScenarios.CHAT + "/.pkg-origin"));
                if (!origin.contains("verified publisher") || origin.contains("unverified")) return "installed as " + origin;
                r.say("\u00a77  Discord installed from " + com.example.evanscomputermod.computer.VillageRepo.NAME + ": verified publisher");
                return null;
            } catch (Exception e) {
                return "no .pkg-origin: " + e;
            }
        }, "Discord is installed (verified publisher)", 30_000);
        b.await(r -> CellPhoneScenarios.tapIn(r, "App Store", "Open"), "tap Open", 15_000);
        b.await(r -> PhoneGuiScenarios.onScreen(r, CellPhoneScenarios.CHAT) != null ? null : "windows " + PhoneGuiScenarios.comp(r).windows(),
                "Discord opens full screen", 20_000);
        // Discord looks for backends on the network as it opens (docs/DISCOVERY.md): the phone's
        // gateway, the carrier core, relays the search to its village's ISP router, which answers
        // for the ring's Discord backend.
        b.await(r -> String.join(" ", PhoneGuiScenarios.comp(r).layerTexts("overlay")).contains("network") ? null
                : "overlay " + PhoneGuiScenarios.comp(r).layerTexts("overlay"), "Discord looks on the network: the phone asks to allow it the network", 20_000);
        PhoneGuiScenarios.tapShell(b, "overlay", "Allow", "tap Allow");
        b.await(r -> {
            String why = CellPhoneScenarios.chatShows(r, com.example.evanscomputermod.computer.VillageDiscovery.DISCORD_NAME);
            if (why == null) r.say("\u00a77  Discord found the ring's backend: " + com.example.evanscomputermod.computer.VillageDiscovery.DISCORD_NAME);
            return why;
        }, "Discord connects to the ring's backend it found (over LTE through the core and the ISP): the Tech Village server", 30_000);
        b.await(r -> CellPhoneScenarios.tapIn(r, "Discord", "# general"), "tap # general", 10_000);
        b.await(r -> CellPhoneScenarios.tapIn(r, "Discord", "Message #general"), "tap the message box", 10_000);
        b.mutate(r -> PhoneGuiScenarios.type(r, "/nick " + CHAT_NICK), "type /nick " + CHAT_NICK);
        b.await(r -> CellPhoneScenarios.tapIn(r, "Discord", "Send"), "tap Send", 5_000);
        b.await(r -> CellPhoneScenarios.tapIn(r, "Discord", "Message #general"), "tap the message box", 10_000);
        b.mutate(r -> PhoneGuiScenarios.type(r, CHAT_HELLO), "type \"" + CHAT_HELLO + "\"");
        b.await(r -> CellPhoneScenarios.tapIn(r, "Discord", "Send"), "tap Send", 5_000);
        b.await(r -> {
            int k = WorldNetwork.get(r.level()).chatVillage();
            String scr = infraScreen(r, k, WorldNetwork.CHAT);
            if (!scr.contains("<" + CHAT_NICK + "> " + CHAT_HELLO)) return "Discord backend (village " + k + "):\n" + scr;
            r.say("\u00a77  the Discord backend in village " + k + " logged: #general <" + CHAT_NICK + "> " + CHAT_HELLO);
            return null;
        }, "the ring's Discord backend logs the phone's message in #general", 15_000);
    }

    // ------------------------------------------------------------ the scenario

    static Scenario techVillage() {
        var b = Scenario.builder("cell_tech_village",
                        "the Tech Village carrier network: a phone from a Phone Store's display attaches to the village's cell site, browses"
                                + " the Data Center over LTE, installs Discord from the village repository its App Store already lists and talks,"
                                + " and keeps its address along the fiber to the next village; a chest SIM in a second phone attaches too")
                .asPlayer()
                .realTime()
                .inWorld()
                .timeLimit(900_000);
        b.note("Stand in a Tech Village's Phone Store (beside the Data Center). Built by the world: the store's cell site in its"
                + " closet, its LTE panel on the tower behind, the display phones and the SIM chest; the carrier core runs in the"
                + " data center of one village; cell sites stand along the fiber to the next village.");
        b.await(r -> {
            State s = st(r);
            return s.va.datacenter() == null ? "no Phone Store in village " + s.a : null;
        }, "find this village's Phone Store and the next village's", 5_000);
        b.note("Take a phone from the display table: hit its item frame, pick it up");
        b.await(CellVillageScenarios::takeDisplayPhone, "the display phone is in the inventory", 10_000);
        b.mutate(CellVillageScenarios::holdAndUse, "hold the phone and use it once (it gets its computer)");
        b.await(PhoneScenarios::booted, "the phone has booted", 60_000);
        b.await(CellPhoneScenarios::celldRunning, "celld runs (an LTE Modem and a SIM are in)", 20_000);
        b.await(r -> CellPhoneScenarios.statusHas(r, "\ncarrier " + plan(r).carrier + "\n"), "the status bar shows the carrier (attached)", 30_000);
        PhoneScenarios.openTerminal(b);
        PhoneScenarios.phoneRun(b, "cell", "^\\s+state\\s+connected", "^cell: celld is not running", "cell: connected", 20_000);
        b.await(r -> {
            State s = st(r);
            String cell = joined(r, "cell\\s+\\S+ cell (\\d+),");
            s.ip = joined(r, "address\\s+(100\\.80\\.\\d+\\.\\d+)/");
            if (cell == null || s.ip == null) return "phone screen:\n" + PhoneScenarios.screen(r);
            r.say("§7  on cell " + cell + " (village " + s.a + "'s Phone Store is cell " + s.a + "), address " + s.ip);
            return null;
        }, "the phone's cell and its address from the core's pool", 5_000);
        b.note("Browse the village's web server in the Data Center over LTE");
        PhoneScenarios.clearScreen(b);
        b.mutate(r -> PhoneScenarios.type(r, PhoneScenarios.holder(r), "curl http://100." + (64 + st(r).a) + ".0.10/\n"),
                "type curl http://100.(64+a).0.10/");
        // The page's end is on screen: its list of the other villages, which never links village a itself.
        b.await(r -> {
            String scr = PhoneScenarios.screen(r);
            int last = st(r).a == 10 ? 9 : 10;   // the list's last entry, still on screen
            boolean page = scr.contains("</body></html>") && scr.contains(">Tech Village " + last + "</a>")
                    && !scr.contains(">Tech Village " + st(r).a + "</a>");
            return page ? null : "phone screen:\n" + scr;
        }, "village a's page arrives over LTE (its list of villages leaves out a)", 30_000);
        appStoreChat(b);
        b.mutate(r -> PhoneGuiScenarios.altKey(r, PhoneGuiScenarios.HID_H), "Alt+H: home (Discord goes to the background)");
        PhoneScenarios.openTerminal(b);
        b.note("[scripted] Travel along the fiber to the next village's Phone Store; the phone hands over from site to site");
        b.await(CellVillageScenarios::ride, "ride along the fiber to village b's Phone Store", 600_000);
        PhoneScenarios.phoneRun(b, "cell", "^\\s+state\\s+connected", "^cell: celld is not running", "cell: still connected at village b", 30_000);
        b.await(r -> {
            State s = st(r);
            String ip = joined(r, "address\\s+(100\\.80\\.\\d+\\.\\d+)/");
            String cell = joined(r, "cell\\s+\\S+ cell (\\d+),");
            String handovers = joined(r, "handovers (\\d+)");
            if (ip == null || cell == null) return "phone screen:\n" + PhoneScenarios.screen(r);
            if (!ip.equals(s.ip)) {
                r.fail("the phone's address changed on the way: " + s.ip + " -> " + ip);
                return null;
            }
            String log = celldLog(r);
            int ho = count(log, "^.*handover cell ") - s.handoversAtStart;
            if (ho < 1) {
                r.fail("no handover on the way (" + s.report + "); celld log:\n" + log.substring(Math.max(0, log.length() - 3000)));
                return null;
            }
            r.say("§7  at village " + s.b + ": cell " + cell + ", address " + ip + " (the same), handovers " + handovers + "; " + s.report);
            return null;
        }, "the same address as at village a, after handovers", 5_000);
        b.note("A second phone: a SIM from the store's chest and an LTE Modem in a new ePhone");
        b.mutate(r -> CellPhoneScenarios.shuffle(r, 0, 30), "put the first phone away (into the backpack: it turns off)");
        b.await(CellVillageScenarios::takeChestSim, "a SIM from the chest", 5_000);
        b.mutate(CellVillageScenarios::secondPhone, "a new ePhone in hand (used once), an LTE Modem in the inventory");
        b.await(PhoneScenarios::booted, "the second phone has booted", 60_000);
        b.mutate(CellPhoneScenarios::fitModemAndSim, "its slot screen: the LTE Modem and the chest SIM in");
        b.await(CellPhoneScenarios::celldRunning, "celld runs on the second phone", 20_000);
        b.await(r -> CellPhoneScenarios.statusHas(r, "\ncarrier " + plan(r).carrier + "\n"), "the second phone is attached", 30_000);
        PhoneScenarios.openTerminal(b);
        PhoneScenarios.phoneRun(b, "cell", "^\\s+state\\s+connected", "^cell: celld is not running", "cell: connected", 20_000);
        b.await(r -> {
            String imsi = joined(r, "IMSI (\\d{15})");
            if (imsi == null) return "phone screen:\n" + PhoneScenarios.screen(r);
            if (!imsi.equals(st(r).imsi)) {
                r.fail("the second phone uses SIM " + imsi + ", not the chest's " + st(r).imsi);
                return null;
            }
            r.say("§7  the second phone is on the network with the chest SIM " + imsi);
            return null;
        }, "the second phone's SIM is the chest's", 5_000);
        b.mutate(CellVillageScenarios::release, "(the ticking area around the player ends)");
        return b.build();
    }

    static {
        for (Scenario s : List.of(techVillage())) ALL.put(s.name, s);
    }
}
//?}
