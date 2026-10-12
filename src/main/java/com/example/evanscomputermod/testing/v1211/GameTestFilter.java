package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import net.minecraft.gametest.framework.GameTestServer;
import net.minecraft.gametest.framework.TestFunction;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.event.server.ServerAboutToStartEvent;

import java.util.Arrays;
import java.util.List;

/**
 * Runs only some tests of a namespace: {@code -Decm.gametest.only=wifi_connect,gui_desktop}
 * (from {@code scripts/Test.ps1 -GameTestOnly ...}) keeps the test functions
 * whose name ends with one of the given names. The GameTest server copies the
 * registered tests when it is created and groups them into batches when it
 * starts, so they are filtered in between ({@code ServerAboutToStartEvent}).
 * The runner counts the tests each batch announces, so a filtered run still
 * needs one pass marker per test it ran. Without the property every test runs.
 */
@EventBusSubscriber(modid = EvansComputerMod.MODID)
public final class GameTestFilter {
    private GameTestFilter() {}

    @SubscribeEvent
    @SuppressWarnings("unchecked")
    static void onServerAboutToStart(ServerAboutToStartEvent event) {
        String only = System.getProperty("ecm.gametest.only", "").trim();
        if (only.isEmpty() || !(event.getServer() instanceof GameTestServer server)) return;
        List<String> names = Arrays.stream(only.split(",")).map(String::trim).filter(s -> !s.isEmpty()).toList();
        try {
            var f = GameTestServer.class.getDeclaredField("testFunctions");
            f.setAccessible(true);
            List<TestFunction> tests = (List<TestFunction>) f.get(server);
            int before = tests.size();
            tests.removeIf(t -> names.stream().noneMatch(n -> t.testName().endsWith("." + n) || t.testName().equals(n)));
            EvansComputerMod.LOGGER.info("GameTest filter {}: {} of {} tests kept", names, tests.size(), before);
        } catch (ReflectiveOperationException | ClassCastException e) {
            EvansComputerMod.LOGGER.error("GameTest filter {} not applied: {}", names, e.toString());
        }
    }
}
//?}
