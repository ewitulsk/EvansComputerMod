package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.testing.scenario.CellularScenarios;
import com.example.evanscomputermod.testing.scenario.RadioScenarios;
import com.example.evanscomputermod.testing.scenario.Scenario;
import com.example.evanscomputermod.testing.scenario.ScenarioRun;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * Cellular v2 (docs/gui/CELLULAR_V2.md), namespace {@code ecm_cellular} (1.21.1): the
 * {@link CellularScenarios}, {@code cell_site_core} and the phone scenarios on real
 * player-built networks, each one batch, each removed once it passes (sites keep
 * beaconing otherwise). Structure {@code gametest_cellular} (48x12x36);
 * {@code cell_lte_far} also builds a site 300 blocks east, kept loaded with
 * /forceload. Sable must be loaded (the roaming cart is a Sable ship).
 */
@GameTestHolder(CellularTests.NS)
@PrefixGameTestTemplate(false)
public final class CellularTests {
    static final String NS = "ecm_cellular";
    static final String STRUCTURE = "gametest_cellular";

    /** LTE to a player-built site 300 blocks away: attach and pings to a server on the core's LAN; no SIM = no service. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".lte_far")
    public static void cell_lte_far(GameTestHelper h) {
        run(h, "cell_lte_far");
    }

    /** mmWave attaches in line of sight and hears nothing behind an earth ridge; with an LTE Modem it gets through. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".mmwave_hill")
    public static void cell_mmwave_hill(GameTestHelper h) {
        run(h, "cell_mmwave_hill");
    }

    /** Two sites behind different routers, one core: a phone on a moving cart keeps its address and an SSH session. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".roaming")
    public static void cell_roaming(GameTestHelper h) {
        run(h, "cell_roaming");
    }

    /** A SIM of another carrier never attaches (the site sees no phone); the right SIM does. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".wrong_carrier")
    public static void cell_wrong_carrier(GameTestHelper h) {
        run(h, "cell_wrong_carrier");
    }

    /**
     * Cellular v2 phase 3, network side: a player builds a core (cored, a Card Reader/Writer, a
     * server on its LAN) and a cell site (celld-site, Advanced SDR, LTE panel on a mast); a wrong
     * site credential is refused, the right one registers over the sealed backhaul, a SIM is
     * written, a framed SDR hears the beacons, the Carrier Console shows the site.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".cell_site_core")
    public static void cell_site_core(GameTestHelper h) {
        run(h, "cell_site_core");
    }

    /**
     * Cellular v2 phase 3, the whole network: cored (with its card writer) and celld-site on player-built computers; an
     * ePhone with an LTE Modem and the SIM cored wrote attaches through celld and pings a server on the core's LAN.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".cell_phone_attach")
    public static void cell_phone_attach(GameTestHelper h) {
        run(h, com.example.evanscomputermod.testing.scenario.CellPhoneScenarios.ALL.get("cell_phone_attach"));
    }

    /** A phone installs Discord from the App Store over LTE, talks with a server in #general, gets a notification in the background. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".cell_chat")
    public static void cell_chat(GameTestHelper h) {
        run(h, com.example.evanscomputermod.testing.scenario.CellPhoneScenarios.ALL.get("cell_chat"));
    }

    /**
     * Cellular v2 phase 5: a Tech Village generated at its planned site (structures are off in the
     * GameTest world, so it is placed the way {@code /place structure} does; the client suite
     * covers natural generation). Its Phone Store holds four ePhones in item frames on the table
     * (LTE Modem, the village's phone SIMs) and a chest of 27 SIMs, every key the one the carrier
     * core's database holds, none in the network encoding clients get (frames, chest); the store's
     * cell site is provisioned for the core, cabled to the data center LAN, its SDR's coax reaches
     * the LTE panel. Then the ring's infrastructure boots (the village's on its blocks, the rest
     * headless, the core in its village) and the store site registers with the core over the
     * ring: {@code celld-site status} says {@code link up}, {@code cored status} lists it.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".tech_store")
    public static void tech_village_phone_store(GameTestHelper h) {
        long start = System.currentTimeMillis();
        String[] failure = {null};
        int[] phase = {0};
        long[] at = {0, 0};
        Object[] state = {null};
        var level = h.getLevel();
        var d = com.example.evanscomputermod.computer.WorldNetwork.get(level);
        TestDriver.drive(h, NS, "tech_village_phone_store", () -> {
            try {
                d.plan(level);
                var plan = d.cells(level);
                if (phase[0] == 0) {
                    var startPiece = com.example.evanscomputermod.worldgen.TechVillageLocator.placeForTest(level, 1);
                    var visit = com.example.evanscomputermod.worldgen.TechVillageLocator.resolve(level, 1, startPiece);
                    com.example.evanscomputermod.worldgen.TechVillageLocator.hold(level, startPiece, true);
                    state[0] = visit;
                    phase[0] = 1;
                    at[0] = System.currentTimeMillis();
                    return false;
                }
                var visit = (com.example.evanscomputermod.worldgen.TechVillageLocator.Visit) state[0];
                var site = (com.example.evanscomputermod.block.TerminalBlockEntity) level.getBlockEntity(
                        com.example.evanscomputermod.worldgen.TechVillageLocator.role(level, visit, com.example.evanscomputermod.computer.CellPlan.STORE_ROLE));
                if (phase[0] == 1) {
                    if (System.currentTimeMillis() - at[0] < 1500) return false;   // the SDR's chain is traced on its ticks
                    java.util.List<String> details = new java.util.ArrayList<>();
                    String why = com.example.evanscomputermod.testing.TechCellChecks.store(level, d, visit, 1, details);
                    if (why == null) {
                        d.provision(level, plan.coreVillage, com.example.evanscomputermod.computer.WorldNetwork.CORE);
                        why = com.example.evanscomputermod.testing.TechCellChecks.coreDb(level, d);
                    }
                    if (why != null) {
                        failure[0] = why;
                        return false;
                    }
                    EvansComputerMod.LOGGER.info("tech_village_phone_store: carrier {} core village {} channel {}; {}", plan.carrier,
                            plan.coreVillage, plan.channel, String.join("; ", details));
                    // Village 1's computers on their blocks; the rest of the ring's infrastructure headless.
                    d.applyLinks(level);
                    for (var v : d.villages)
                        for (String role : d.infrastructure(v.number())) {
                            if (v.number() == 1) {
                                var pos = com.example.evanscomputermod.worldgen.TechVillageLocator.role(level, visit, role);
                                if (pos != null && level.getBlockEntity(pos) instanceof com.example.evanscomputermod.block.TerminalBlockEntity be)
                                    be.initializeWasm();
                            } else d.boot(level, v.number(), role);
                        }
                    site.initializeWasm();
                    at[0] = System.currentTimeMillis();
                    phase[0] = 2;
                    return false;
                }
                if (site.getComputer() == null) return timeout(start, failure, "the store site did not boot");
                String screen = ScenarioRun.screen(site.getDisplay());
                // The core: on its block when it lives in village 1, else headless.
                com.example.evanscomputermod.computer.ComputerInstance core;
                String coreScreen;
                if (plan.coreVillage == 1) {
                    var be = (com.example.evanscomputermod.block.TerminalBlockEntity) level.getBlockEntity(com.example.evanscomputermod.worldgen.TechVillageLocator
                            .role(level, visit, com.example.evanscomputermod.computer.WorldNetwork.CORE));
                    core = be.getComputer();
                    coreScreen = ScenarioRun.screen(be.getDisplay());
                } else {
                    var coreHost = com.example.evanscomputermod.computer.ComputerHost.get(level.getServer(),
                            d.identity(level, plan.coreVillage, com.example.evanscomputermod.computer.WorldNetwork.CORE));
                    core = coreHost.instance();
                    coreScreen = ScenarioRun.screen(coreHost.headlessDisplay());
                }
                if (phase[0] == 2) {
                    if (java.util.regex.Pattern.compile("(?m)^link up").matcher(screen).find()) {
                        EvansComputerMod.LOGGER.info("tech_village_phone_store: store site link up {} ms after boot", System.currentTimeMillis() - at[0]);
                        phase[0] = 3;
                        at[1] = 0;
                        return false;
                    }
                    if (System.currentTimeMillis() > at[1] && screen.contains("/ >")) {
                        site.getComputer().sendInput("celld-site status\n");
                        at[1] = System.currentTimeMillis() + 4000;
                    }
                    return timeout(start, failure, "the store site never registered with the core:\n" + screen.stripTrailing()
                            + "\n--- core ---\n" + coreScreen.stripTrailing());
                }
                if (core != null && java.util.regex.Pattern.compile("(?m)^site v1-store 100\\.65\\.0\\.40 ").matcher(coreScreen).find()) {
                    EvansComputerMod.LOGGER.info("tech_village_phone_store: core lists the site:\n{}", coreScreen.stripTrailing());
                    return true;
                }
                if (core != null && System.currentTimeMillis() > at[1]) {
                    core.sendInput("cored status\n");
                    at[1] = System.currentTimeMillis() + 4000;
                }
                return timeout(start, failure, "cored does not list v1-store connected:\n" + coreScreen.stripTrailing());
            } catch (Exception e) {
                failure[0] = e.toString();
                EvansComputerMod.LOGGER.error("tech_village_phone_store", e);
                return false;
            }
        }, () -> failure[0]);
    }

    /**
     * The Tech Village package repository (docs/TECH_VILLAGE_NETWORK.md "Apps"): the chat village
     * generated at its planned site (placed the way {@code /place structure} does). Every computer
     * it generated and every display phone of its Phone Store has the repository in
     * {@code /etc/pkg/repos} with the world's key pinned, and none has Chat installed. The package
     * server on its rack boots, {@code pkg serve} publishes a valid index signed with that key that
     * lists only Chat, and the village's web server, on the same Data Center LAN, finds Chat there
     * with {@code pkg search} (its repository list came with the village; the index over HTTP). Then the rest of the ring's
     * infrastructure (headless) has the repository too and no Chat.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".tech_repo")
    public static void tech_village_repo(GameTestHelper h) {
        long start = System.currentTimeMillis();
        String[] failure = {null};
        int[] phase = {0};
        long[] at = {0, 0};
        Object[] state = {null};
        var level = h.getLevel();
        var d = com.example.evanscomputermod.computer.WorldNetwork.get(level);
        TestDriver.drive(h, NS, "tech_village_repo", () -> {
            try {
                d.plan(level);
                var repo = d.repo(level);
                int k = repo.village();
                if (phase[0] == 0) {
                    var startPiece = com.example.evanscomputermod.worldgen.TechVillageLocator.placeForTest(level, k);
                    var visit = com.example.evanscomputermod.worldgen.TechVillageLocator.resolve(level, k, startPiece);
                    com.example.evanscomputermod.worldgen.TechVillageLocator.hold(level, startPiece, true);
                    state[0] = visit;
                    java.util.List<String> details = new java.util.ArrayList<>();
                    String why = com.example.evanscomputermod.testing.TechCellChecks.repoConfigured(level, d, visit, k, details);
                    if (why != null) {
                        failure[0] = why;
                        return false;
                    }
                    EvansComputerMod.LOGGER.info("tech_village_repo: {} in village {}; {}", repo, k, String.join("; ", details));
                    d.applyLinks(level);
                    for (String role : java.util.List.of(com.example.evanscomputermod.computer.WorldNetwork.REPO,
                            com.example.evanscomputermod.computer.WorldNetwork.WEB)) {
                        var pos = com.example.evanscomputermod.worldgen.TechVillageLocator.role(level, visit, role);
                        if (pos == null || !(level.getBlockEntity(pos) instanceof com.example.evanscomputermod.block.TerminalBlockEntity be)
                                || !be.getComputerId().equals(d.identity(level, k, role))) {
                            failure[0] = "no " + role + " computer on its rack in village " + k + " (" + pos + ")";
                            return false;
                        }
                        be.initializeWasm();
                    }
                    phase[0] = 1;
                    at[0] = System.currentTimeMillis();
                    return false;
                }
                var visit = (com.example.evanscomputermod.worldgen.TechVillageLocator.Visit) state[0];
                var server = (com.example.evanscomputermod.block.TerminalBlockEntity) level.getBlockEntity(
                        com.example.evanscomputermod.worldgen.TechVillageLocator.role(level, visit, com.example.evanscomputermod.computer.WorldNetwork.REPO));
                var web = (com.example.evanscomputermod.block.TerminalBlockEntity) level.getBlockEntity(
                        com.example.evanscomputermod.worldgen.TechVillageLocator.role(level, visit, com.example.evanscomputermod.computer.WorldNetwork.WEB));
                if (server.getComputer() == null || web.getComputer() == null) return timeout(start, failure, "the package server or the web server did not boot");
                String serverScreen = ScenarioRun.screen(server.getDisplay());
                if (phase[0] == 1) {
                    if (!java.util.regex.Pattern.compile("Serving \\d+ packages from /srv/repo on port 8080").matcher(serverScreen).find())
                        return timeout(start, failure, "pkg serve is not serving the repository:\n" + serverScreen.stripTrailing());
                    java.util.List<String> details = new java.util.ArrayList<>();
                    String why = com.example.evanscomputermod.testing.TechCellChecks.repoIndex(level, d, details);
                    if (why != null) {
                        failure[0] = why;
                        return false;
                    }
                    EvansComputerMod.LOGGER.info("tech_village_repo: pkg serve up {} ms after boot; {}", System.currentTimeMillis() - at[0],
                            String.join("; ", details));
                    phase[0] = 2;
                    at[1] = 0;
                    return false;
                }
                String webScreen = ScenarioRun.screen(web.getDisplay());
                if (phase[0] == 2) {
                    // The web server's pkg (its /etc/pkg/repos came with the village) fetches the index over HTTP.
                    if (webScreen.contains("dev.ecm.discord 1.0.0  Discord  [Tech-Village-Apps, signed]") && serverScreen.contains("GET /index.json")) {
                        EvansComputerMod.LOGGER.info("tech_village_repo: the web server fetched the index:\n{}\n--- package server ---\n{}",
                                webScreen.stripTrailing(), serverScreen.stripTrailing());
                        phase[0] = 3;
                        at[1] = 0;
                        return false;
                    }
                    if (System.currentTimeMillis() > at[1] && webScreen.contains("/ >")) {
                        web.getComputer().sendInput("pkg search\n");
                        at[1] = System.currentTimeMillis() + 6000;
                    }
                    return timeout(start, failure, "pkg search on the web server did not list Discord from the repository:\n" + webScreen.stripTrailing() + "\n--- package server ---\n"
                            + serverScreen.stripTrailing());
                }
                if (phase[0] == 3) {
                    // Service discovery (docs/DISCOVERY.md) on the Data Center LAN: the package server's
                    // disco serve answers for the repository pkg serve announced, under its name.
                    String found = "pkg   " + repo.address() + ":" + com.example.evanscomputermod.computer.VillageRepo.PORT;
                    if (java.util.regex.Pattern.compile(java.util.regex.Pattern.quote(found) + "\\s+" + com.example.evanscomputermod.computer.VillageRepo.NAME)
                            .matcher(webScreen).find()) {
                        EvansComputerMod.LOGGER.info("tech_village_repo: the web server found the repository with disco:\n{}", webScreen.stripTrailing());
                        phase[0] = 4;
                        return false;
                    }
                    if (System.currentTimeMillis() > at[1] && webScreen.contains("/ >")) {
                        web.getComputer().sendInput("disco pkg\n");
                        at[1] = System.currentTimeMillis() + 6000;
                    }
                    return timeout(start, failure, "disco pkg on the web server did not find the package server:\n" + webScreen.stripTrailing()
                            + "\n--- package server ---\n" + serverScreen.stripTrailing());
                }
                // The rest of the ring's infrastructure, provisioned as at server start: the repository and no Chat.
                for (var v : d.villages)
                    for (String role : d.infrastructure(v.number())) {
                        d.provision(level, v.number(), role);
                        String why = com.example.evanscomputermod.testing.TechCellChecks.repoOn(level, repo, d.identity(level, v.number(), role),
                                "village " + v.number() + " " + role);
                        if (why != null) {
                            failure[0] = why;
                            return false;
                        }
                    }
                // Booted computers didn't install Chat either (only the system apps).
                for (var be : new com.example.evanscomputermod.block.TerminalBlockEntity[] {server, web})
                    if (be.getComputer().getAppManager().manifest(com.example.evanscomputermod.testing.TechCellChecks.CHAT_APP) != null) {
                        failure[0] = "Chat is installed on " + be.getComputerId();
                        return false;
                    }
                return true;
            } catch (Exception e) {
                failure[0] = e.toString();
                EvansComputerMod.LOGGER.error("tech_village_repo", e);
                return false;
            }
        }, () -> failure[0]);
    }

    private static boolean timeout(long start, String[] failure, String why) {
        if (System.currentTimeMillis() - start > 55_000) failure[0] = why;
        return false;
    }

    /** {@link TestDriver#scenario} with the last screen lines of every computer added to a failure report. */
    private static void run(GameTestHelper h, String name) {
        run(h, RadioScenarios.ALL.get(name));
    }

    private static void run(GameTestHelper h, Scenario s) {
        ScenarioRun r = TestDriver.build(h, s, s.name);
        long[] start = {0}, ticks = {0};
        String[] why = {null};
        TestDriver.drive(h, NS, s.name, () -> {
            if (s.realTime) {
                long now = System.nanoTime();
                if (start[0] == 0) start[0] = now;
                long due = start[0] + ++ticks[0] * 50_000_000L;
                if (due > now) java.util.concurrent.locks.LockSupport.parkNanos(due - now);
            }
            if (r.tick() != ScenarioRun.State.PASSED) return false;
            r.clear();
            return true;
        }, () -> {
            if (r.state() != ScenarioRun.State.FAILED) return null;
            if (why[0] == null) {
                why[0] = r.failure() + "\n" + CellularScenarios.diagnoseRun(r) + "\n" + r.dump();
                EvansComputerMod.LOGGER.error("[{}] {}", s.name, why[0]);
                r.clear();
            }
            return why[0];
        });
    }

    private CellularTests() {}
}
//?}
