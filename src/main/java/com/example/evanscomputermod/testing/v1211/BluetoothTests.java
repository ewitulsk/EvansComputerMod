package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.AirPodsScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * Bluetooth and AirPods, namespace {@code ecm_bluetooth}: the same definition
 * {@code /ecm scenario spawn airpods} builds.
 */
@GameTestHolder(BluetoothTests.NS)
@PrefixGameTestTemplate(false)
public final class BluetoothTests {
    static final String NS = "ecm_bluetooth";
    private static final String STRUCTURE = "gametest_empty";

    /** Pair AirPods from Settings > Bluetooth (lid open), wear them: phone audio in the ears, its microphone theirs. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".airpods")
    public static void airpods(GameTestHelper h) {
        TestDriver.scenario(h, NS, AirPodsScenarios.ALL.get("airpods"), true);
    }
}
//?}
