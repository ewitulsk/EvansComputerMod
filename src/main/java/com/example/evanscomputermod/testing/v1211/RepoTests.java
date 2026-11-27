package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.RepoScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * TLS and the package repository (docs/TLS.md, docs/REPO.md), namespace
 * {@code ecm_repo} (1.21.1): the {@link RepoScenarios}, player-built and
 * typed, one batch each. None of these needs anything outside the game.
 */
@GameTestHolder(RepoTests.NS)
@PrefixGameTestTemplate(false)
public final class RepoTests {
    static final String NS = "ecm_repo";

    /** An authority signs a certificate after its check; HTTPS works for a client that trusts it, and only then. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".tls_web")
    public static void tls_web(GameTestHelper h) {
        TestDriver.scenario(h, NS, RepoScenarios.ALL.get("tls_web"), true);
    }

    /** A library, an app needing it, a program and a source package: installed, used from Python and Lua, held while needed. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".repo_kinds")
    public static void repo_kinds(GameTestHelper h) {
        TestDriver.scenario(h, NS, RepoScenarios.ALL.get("repo_kinds"), true);
    }

    /** A data center mirrors a chosen part of an HTTPS repository; a client installs from it with the origin's key pinned. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".repo_mirror")
    public static void repo_mirror(GameTestHelper h) {
        TestDriver.scenario(h, NS, RepoScenarios.ALL.get("repo_mirror"), true);
    }

    private RepoTests() {}
}
//?}
