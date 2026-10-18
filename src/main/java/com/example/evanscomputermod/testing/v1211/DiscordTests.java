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
    /**
     * bo's computer is 30 blocks from the test's own (force-loaded) chunks, so, depending on
     * where the test lands, its chunk can be loaded without ticking (and its Speaker and
     * Microphone never attach). In a world a player within simulation distance keeps it
     * ticking; here a timed ticket does (it expires on its own after five minutes).
     */
    private static final net.minecraft.server.level.TicketType<net.minecraft.world.level.ChunkPos> NEAR_PLAYER =
            net.minecraft.server.level.TicketType.create("ecm_discord_near_player",
                    java.util.Comparator.comparingLong(net.minecraft.world.level.ChunkPos::toLong), 6000);

    /** Keep the chunks of every computer ticking, as a player standing among them would. */
    private static void keepTicking(com.example.evanscomputermod.testing.scenario.ScenarioRun r) {
        for (var p : java.util.List.of(DiscordScenarios.SERVER, DiscordScenarios.ALEX, DiscordScenarios.BO)) {
            var c = new net.minecraft.world.level.ChunkPos(r.abs(p));
            r.level().getChunkSource().addRegionTicket(NEAR_PLAYER, c, 2, c);
        }
    }

    /** Two backends found by themselves; a server made on one with text and voice channels; another player joins and posts. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".servers")
    public static void discord_servers(GameTestHelper h) {
        TestDriver.scenario(h, NS, DiscordScenarios.ALL.get("discord_servers"), true, DiscordTests::keepTicking);
    }

    /** A voice channel: what alex's Microphone hears plays on bo's Speaker (not before; not while muted). */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".voice")
    public static void discord_voice(GameTestHelper h) {
        TestDriver.scenario(h, NS, DiscordScenarios.ALL.get("discord_voice"), true, DiscordTests::keepTicking);
    }

    /** Accounts: the Auth server reinstalled with a new key; Discord and chat refuse it until the old pin is removed by hand. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".auth_pinned_key")
    public static void auth_pinned_key(GameTestHelper h) {
        TestDriver.scenario(h, NS, com.example.evanscomputermod.testing.scenario.AuthScenarios.ALL.get("auth_pinned_key"), true, DiscordTests::keepTicking);
    }

    /** Accounts: one Auth server for two backends (one sign-up), another realm asks again, Music keeps its own login. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".auth_shared_realm")
    public static void auth_shared_realm(GameTestHelper h) {
        TestDriver.scenario(h, NS, com.example.evanscomputermod.testing.scenario.AuthScenarios.ALL.get("auth_shared_realm"), true, DiscordTests::keepTicking);
    }
}
//?}
