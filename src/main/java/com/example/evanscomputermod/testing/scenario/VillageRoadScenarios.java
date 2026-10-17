package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.WorldNetwork;
import com.example.evanscomputermod.testing.TechCableChecks;
import com.example.evanscomputermod.testing.TechRoadChecks;
import com.example.evanscomputermod.worldgen.TechVillageLocator;

import java.util.*;

/**
 * Tech Village roads as a player meets them: standing in a generated Tech Village (for example
 * after {@code /ecm village tp <n>}), the player walks out of the ISP's front door, the Data
 * Center's door and the Phone Store's door, and each time follows the road (path blocks,
 * stepping up or down a block) until it joins one of the village's streets. The roads come from
 * world generation; the scenario only reads blocks ({@link TechRoadChecks}). The
 * {@code ecm_router} GameTest {@code tech_village_roads} runs this definition on a village it
 * generates (GameTest worlds have structures off); the {@code tech} client suite checks the
 * same in fresh, naturally generated worlds.
 */
public final class VillageRoadScenarios {
    private VillageRoadScenarios() {}

    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();
    /** The village a GameTest generated for its run (GameTest worlds have none); otherwise the nearest one. */
    public static final Map<ScenarioRun, Integer> VILLAGE = Collections.synchronizedMap(new WeakHashMap<>());

    static {
        var b = Scenario.builder("tech_village_roads",
                        "every Tech Village building (ISP, Data Center, Phone Store, and any added later) has a road from its door"
                                + " to the village streets")
                .asPlayer()
                .inWorld();
        b.note("Stand in a Tech Village (/ecm village tp <n>; the nearest village is used). Built by the world: a road from each"
                + " building's door to the streets. Expected: every door reports \"street in N blocks\". Failure: a door"
                + " reports \"no road to a street\" (the building is cut off), or a mod building has doors but no entrance"
                + " marker. Control: the vanilla houses are untouched; the ISP's front path was already a street end.");
        b.await(r -> {
            var d = WorldNetwork.get(r.level());
            d.plan(r.level());
            Integer chosen = VILLAGE.get(r);
            int n = chosen != null ? chosen : d.nearest(r.origin()).number();
            TechVillageLocator.Visit v;
            try {
                v = TechVillageLocator.visit(r.level(), n);
            } catch (IllegalStateException e) {
                return e.getMessage();
            }
            List<String> details = new ArrayList<>();
            String why = TechRoadChecks.check(r.level(), v.start(), details);
            for (String line : details) r.say("§7  village " + n + ": " + line);
            return why;
        }, "walk from every building's door along the road to a street", 10_000);
        ALL.put("tech_village_roads", b.build());

        var c = Scenario.builder("tech_village_cables",
                        "the ISP router's Interface Block and every cable in the ISP and the Data Center visibly join what they"
                                + " are wired to")
                .asPlayer()
                .inWorld();
        c.note("Stand in a Tech Village (/ecm village tp <n>; the nearest village is used). Built by the world: the ISP router"
                + " with its Interface Block, its cables to the mast panels and the Data Center, the racks' cable. Look at the"
                + " router. Expected: the Interface Block beside it shows an arm into the router, each cable on the router's"
                + " faces shows an arm into it, and every cable or Interface Block shows arms exactly toward the blocks it"
                + " connects to. Failure: a block that looks unplugged (no arm) beside something it is wired to, or an arm"
                + " into nothing. Control: the router's screen face never gets an arm.");
        c.await(r -> {
            var d = WorldNetwork.get(r.level());
            d.plan(r.level());
            Integer chosen = VILLAGE.get(r);
            int n = chosen != null ? chosen : d.nearest(r.origin()).number();
            TechVillageLocator.Visit v;
            try {
                v = TechVillageLocator.visit(r.level(), n);
            } catch (IllegalStateException e) {
                return e.getMessage();
            }
            List<String> details = new ArrayList<>();
            String why = TechCableChecks.check(r.level(), v, details);
            for (String line : details) r.say("§7  village " + n + ": " + line);
            return why;
        }, "look at the ISP router, its Interface Block and the cables in the ISP and the Data Center", 10_000);
        ALL.put("tech_village_cables", c.build());
    }
}
//?}
