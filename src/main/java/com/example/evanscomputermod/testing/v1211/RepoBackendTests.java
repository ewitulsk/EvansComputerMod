package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.RepoBackendScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * The official package repository against its real backend (docs/REPO.md),
 * namespace {@code ecm_repo_backend} (1.21.1): the {@link RepoBackendScenarios}.
 * They need the backend running on this machine
 * ({@code scripts/repo-backend-local.sh up}), its settings in the environment,
 * and the internet sidecar in the run (TESTING.md, "Repository backend").
 */
@GameTestHolder(RepoBackendTests.NS)
@PrefixGameTestTemplate(false)
public final class RepoBackendTests {
    static final String NS = "ecm_repo_backend";

    /** repo.access = open: a computer with no cable installs through the server; switched to internet it has no network. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".repo_open")
    public static void repo_open(GameTestHelper h) {
        TestDriver.scenario(h, NS, RepoBackendScenarios.ALL.get("repo_open"), true);
    }

    /** repo.access = internet: a computer cabled to an Internet Gateway installs over its own network. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".repo_internet")
    public static void repo_internet(GameTestHelper h) {
        TestDriver.scenario(h, NS, RepoBackendScenarios.ALL.get("repo_internet"), true);
    }

    /** repo.access = datacenter: a granted repository server mirrors everything and serves it to a cabled client. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".repo_datacenter")
    public static void repo_datacenter(GameTestHelper h) {
        TestDriver.scenario(h, NS, RepoBackendScenarios.ALL.get("repo_datacenter"), true);
    }

    /** The same, mirroring one chosen app: what was not chosen is not offered. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".repo_curated")
    public static void repo_curated(GameTestHelper h) {
        TestDriver.scenario(h, NS, RepoBackendScenarios.ALL.get("repo_curated"), true);
    }

    /** API key sign-in kept in the secret store; publish confirmed in chat; like, favorite, yank, sign out. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".repo_account")
    public static void repo_account(GameTestHelper h) {
        TestDriver.scenario(h, NS, RepoBackendScenarios.ALL.get("repo_account"), true);
    }

    /** pkg login: a code and a chat link; approved in the browser, pkg is signed in. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".repo_login")
    public static void repo_login(GameTestHelper h) {
        TestDriver.scenario(h, NS, RepoBackendScenarios.ALL.get("repo_login"), true);
    }

    private RepoBackendTests() {}
}
//?}
