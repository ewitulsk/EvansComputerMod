package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.DiscordScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * Discord, namespace {@code ecm_discord}: the same definitions
 * {@code /ecm scenario spawn discord_servers|discord_voice} builds.
 */
@GameTestHolder(DiscordTests.NS)
@PrefixGameTestTemplate(false)
public final class DiscordTests {
    static final String NS = "ecm_discord";
    private static final String STRUCTURE = "gametest_empty";

    /** Two backends found by themselves; a server made on one with text and voice channels; another player joins and posts. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".servers")
    public static void discord_servers(GameTestHelper h) {
        TestDriver.scenario(h, NS, DiscordScenarios.ALL.get("discord_servers"), true);
    }

    /** A voice channel: what alex's Microphone hears plays on bo's Speaker (not before; not while muted). */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".voice")
    public static void discord_voice(GameTestHelper h) {
        TestDriver.scenario(h, NS, DiscordScenarios.ALL.get("discord_voice"), true);
    }
}
//?}
