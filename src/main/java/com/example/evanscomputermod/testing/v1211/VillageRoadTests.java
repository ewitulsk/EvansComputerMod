package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.computer.WorldNetwork;
import com.example.evanscomputermod.testing.TechCableChecks;
import com.example.evanscomputermod.testing.TechRoadChecks;
import com.example.evanscomputermod.testing.scenario.VillageRoadScenarios;
import com.example.evanscomputermod.worldgen.TechVillageLocator;
import com.example.evanscomputermod.worldgen.TemplateScan;
import com.example.evanscomputermod.worldgen.mixin.SinglePoolElementAccessor;

import net.minecraft.core.registries.Registries;
import net.minecraft.gametest.framework.*;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.levelgen.structure.pools.SinglePoolElement;
import net.neoforged.neoforge.gametest.*;

import java.util.*;

/**
 * Tech Village roads (namespace {@code ecm_router}, with the other village tests): every building
 * entrance reaches the village streets on a road.
 */
@GameTestHolder("ecm_router")
@PrefixGameTestTemplate(false)
public final class VillageRoadTests {
    private static final String NS = "ecm_router";

    /** The {@code tech_village_roads} scenario on the village nearest the test (generated here: GameTest worlds have no structures). */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".roads")
    public static void tech_village_roads(GameTestHelper h) {
        var level = h.getLevel();
        var d = WorldNetwork.get(level);
        d.plan(level);
        int n = d.nearest(h.absolutePos(net.minecraft.core.BlockPos.ZERO)).number();
        TechVillageLocator.placeForTest(level, n);
        TestDriver.scenario(h, NS, VillageRoadScenarios.ALL.get("tech_village_roads"), false,
                r -> VillageRoadScenarios.VILLAGE.put(r, n));
    }

    /** One village of every style on the ring: each building's entrance reaches a street. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".roads_styles")
    public static void tech_village_roads_every_style(GameTestHelper h) {
        var level = h.getLevel();
        var d = WorldNetwork.get(level);
        d.plan(level);
        Map<String, Integer> first = new LinkedHashMap<>();
        for (var v : d.villages) first.putIfAbsent(v.style(), v.number());
        List<String> failures = new ArrayList<>();
        for (var e : first.entrySet()) {
            var start = TechVillageLocator.placeForTest(level, e.getValue());
            List<String> details = new ArrayList<>();
            String why = TechRoadChecks.check(level, start, details);
            EvansComputerMod.LOGGER.info("tech_village_roads_every_style: village {} ({}): {}", e.getValue(), e.getKey(),
                    String.join("; ", details));
            if (why != null) failures.add("village " + e.getValue() + " (" + e.getKey() + "): " + why);
        }
        String failure = failures.isEmpty() ? null : String.join("\n", failures);
        TestDriver.drive(h, NS, "tech_village_roads_every_style", () -> true, () -> failure);
    }

    /** The {@code tech_village_cables} scenario on the village nearest the test. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".cables")
    public static void tech_village_cables(GameTestHelper h) {
        var level = h.getLevel();
        var d = WorldNetwork.get(level);
        d.plan(level);
        int n = d.nearest(h.absolutePos(net.minecraft.core.BlockPos.ZERO)).number();
        TechVillageLocator.placeForTest(level, n);
        TestDriver.scenario(h, NS, VillageRoadScenarios.ALL.get("tech_village_cables"), false,
                r -> VillageRoadScenarios.VILLAGE.put(r, n));
    }

    /** One village of every style (so several rotations): the ISP's cables join what they are wired to. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".cables_styles")
    public static void tech_village_cables_every_style(GameTestHelper h) {
        var level = h.getLevel();
        var d = WorldNetwork.get(level);
        d.plan(level);
        Map<String, Integer> first = new LinkedHashMap<>();
        for (var v : d.villages) first.putIfAbsent(v.style(), v.number());
        List<String> failures = new ArrayList<>();
        for (var e : first.entrySet()) {
            var start = TechVillageLocator.placeForTest(level, e.getValue());
            var visit = TechVillageLocator.resolve(level, e.getValue(), start);
            List<String> details = new ArrayList<>();
            String why = TechCableChecks.check(level, visit, details);
            EvansComputerMod.LOGGER.info("tech_village_cables_every_style: village {} ({}, {}): {}", e.getValue(), e.getKey(),
                    visit.isp().getRotation(), String.join("; ", details));
            if (why != null) failures.add("village " + e.getValue() + " (" + e.getKey() + "): " + why);
        }
        String failure = failures.isEmpty() ? null : String.join("\n", failures);
        TestDriver.drive(h, NS, "tech_village_cables_every_style", () -> true, () -> failure);
    }

    /**
     * Guard for buildings added later: every building template in the mod's Tech Village pools
     * that has a door also has an entrance marker, so the road planner connects it.
     */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".roads_markers")
    public static void tech_village_buildings_have_entrances(GameTestHelper h) {
        var level = h.getLevel();
        var pools = level.registryAccess().registryOrThrow(Registries.TEMPLATE_POOL);
        List<String> missing = new ArrayList<>(), checked = new ArrayList<>();
        for (var entry : pools.entrySet()) {
            var key = entry.getKey().location();
            if (!key.getNamespace().equals(EvansComputerMod.MODID) || !key.getPath().startsWith("tech_village/")) continue;
            for (var element : entry.getValue().getShuffledTemplates(RandomSource.create(0))) {
                if (!(element instanceof SinglePoolElement single)) continue;
                var loc = ((SinglePoolElementAccessor) single).ecm$template().left().orElse(null);
                if (loc == null || checked.contains(loc.toString())) continue;
                checked.add(loc.toString());
                var scan = TemplateScan.of(level.getStructureManager(), loc).orElse(null);
                if (scan == null) missing.add(loc + " (template missing)");
                else if (scan.hasDoor() && scan.entrances().isEmpty()) missing.add(loc.toString());
            }
        }
        EvansComputerMod.LOGGER.info("tech_village_buildings_have_entrances: checked {}", checked);
        String failure = checked.isEmpty() ? "no Tech Village templates found"
                : missing.isEmpty() ? null : "buildings with doors but no entrance marker: " + missing;
        TestDriver.drive(h, NS, "tech_village_buildings_have_entrances", () -> true, () -> failure);
    }

    /**
     * {@code techVillages.enabled}: on, the ring's placement selects every village's chunk; off, it selects
     * none and the fiber feature places nothing at a fiber end. Control: on again, the chunks are selected again.
     */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".villages_toggle")
    public static void tech_villages_config_toggle(GameTestHelper h) {
        var level = h.getLevel();
        var d = WorldNetwork.get(level);
        d.plan(level);
        var state = level.getChunkSource().getGeneratorState();
        var placement = new com.example.evanscomputermod.worldgen.TechVillagePlacement();
        java.util.function.IntSupplier selected = () -> (int) d.villages.stream()
                .filter(v -> placement.isStructureChunk(state, v.x() >> 4, v.z() >> 4)).count();
        String failure = null;
        try {
            int on = selected.getAsInt();
            com.example.evanscomputermod.EcmConfig.overrideTechVillagesEnabled(false);
            int off = selected.getAsInt();
            var fiberEnd = d.villages.get(0).endpoint(true);
            var feature = com.example.evanscomputermod.worldgen.TechWorldgen.FIBER_LINE.get();
            boolean placed = feature.place(new net.minecraft.world.level.levelgen.feature.FeaturePlaceContext<>(
                    Optional.empty(), level, level.getChunkSource().getGenerator(), RandomSource.create(1),
                    fiberEnd, net.minecraft.world.level.levelgen.feature.configurations.NoneFeatureConfiguration.INSTANCE));
            com.example.evanscomputermod.EcmConfig.overrideTechVillagesEnabled(null);
            int again = selected.getAsInt();
            EvansComputerMod.LOGGER.info("tech_villages_config_toggle: selected on={} off={} again={}, fiber placed off={}",
                    on, off, again, placed);
            if (on != d.villages.size()) failure = "enabled: " + on + " of " + d.villages.size() + " village chunks selected";
            else if (off != 0) failure = "disabled: " + off + " village chunks still selected";
            else if (placed) failure = "disabled: the fiber feature placed blocks";
            else if (again != on) failure = "control: enabled again, " + again + " village chunks selected";
        } finally {
            com.example.evanscomputermod.EcmConfig.overrideTechVillagesEnabled(null);
        }
        String f = failure;
        TestDriver.drive(h, NS, "tech_villages_config_toggle", () -> true, () -> f);
    }
}
//?}
