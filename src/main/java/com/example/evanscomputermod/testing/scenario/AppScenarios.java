package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * App platform scenarios (docs/gui/APPS.md), spawnable with
 * {@code /ecm scenario spawn <name>} and run by the {@code ecm_apps}
 * GameTests. Built and operated by a player: two computers placed and cabled
 * by hand, everything else typed.
 */
public final class AppScenarios {

    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static final String PROMPT = PlayerKit.PROMPT;
    static final String REPO_IP = "10.0.7.1", CLIENT_IP = "10.0.7.2";
    static final String APP = "dev.ecm.hello";

    /** The app's two files, typed into the Python REPL with shell.write_file (shell redirects don't reach programs). */
    static final String[] APP_TOML = {
            "id = \"" + APP + "\"",
            "name = \"Hello\"",
            "version = \"1.0.0\"",
            "entry = \"main.py\"",
            "summary = \"Says hello and tries the network\"",
            "permissions = [\"network\"]",
    };
    static final String[] MAIN_PY = {
            "import net",
            "print(\"hello from " + APP + "\")",
            "try:",
            "    net.tcp_connect(\"" + REPO_IP + "\", 8080)",
            "    print(\"network: ok\")",
            "except Exception:",
            "    print(\"network: denied\")",
    };

    private static String pyString(String... lines) {
        StringBuilder text = new StringBuilder();
        for (String l : lines) text.append(l.replace("\\", "\\\\").replace("\"", "\\\"")).append("\\n");
        return "\"" + text + "\"";
    }

    /**
     * {@code app_repo}: computer {@code repo} writes a small Python app
     * ({@code app.toml} + {@code main.py}, written from the Python REPL), packs it with
     * {@code pkg build} and hosts a repository with {@code pkg serve}. Computer
     * {@code client}, cabled to it, adds the repository, finds the app with
     * {@code pkg search}, installs it over the network and runs it. The app
     * asks for {@code network}; in console mode nobody can be asked, so its
     * connection is refused and {@code pkg run} says how to grant it; after
     * {@code pkg perm grant} the same app connects. Controls: installing with
     * no repository fails; the denied run never connects.
     */
    static Scenario appRepo() {
        var b = Scenario.builder("app_repo",
                        "computer A serves a package repository (pkg build + pkg serve); computer B installs the app over a cable"
                                + " with pkg and runs it; the app is denied network until it is granted")
                .asPlayer()
                .timeLimit(58_000);
        b.host("repo", new BlockPos(0, 1, 0), REPO_IP + "/24");
        b.host("client", new BlockPos(4, 1, 0), CLIENT_IP + "/24");
        b.link("lan", "repo", Direction.DOWN, "client", Direction.DOWN,
                Scenario.Path.from(new BlockPos(0, 0, 0)).go(Direction.EAST, 4));
        b.note("Setup (done for you): two computers, a cable between their bottom faces.");
        b.configureHosts();

        b.note("Control: the client has no repository yet");
        b.send("client", "pkg install " + APP);
        b.expect("client", "^pkg: no repositories", "install refused without a repository");
        b.expect("client", PROMPT, "shell");

        b.note("repo: write the app (app.toml + main.py) from Python, pack it with pkg build, serve it");
        b.send("repo", "mkdir -p /src/hello");
        b.expect("repo", PROMPT, "shell");
        PlayerKit.pythonStart(b, "repo");
        PlayerKit.py(b, "repo", "import shell", "import shell");
        PlayerKit.pyPrint(b, "repo", "shell.write_file(\"/src/hello/app.toml\", " + pyString(APP_TOML) + ")", "True", "app.toml written");
        PlayerKit.pyPrint(b, "repo", "shell.write_file(\"/src/hello/main.py\", " + pyString(MAIN_PY) + ")", "True", "main.py written");
        PlayerKit.pythonEnd(b, "repo");
        b.send("repo", "cat /src/hello/app.toml");
        b.expect("repo", "^permissions = \\[\"network\"\\]", "the manifest is on disk");
        b.expect("repo", PROMPT, "shell");
        b.send("repo", "pkg build /src/hello /srv/repo");
        b.expect("repo", "^Built /srv/repo/dev\\.ecm\\.hello-1\\.0\\.0\\.ecmpkg \\(2 files", "package built", "^pkg: ");
        b.expect("repo", PROMPT, "shell");
        b.send("repo", "pkg serve /srv/repo 8080 &");
        b.expect("repo", "Serving 1 packages from /srv/repo on port 8080", "repository online", "^pkg: ");

        b.note("client: add the repository, search, install");
        b.send("client", "pkg repo add a http://" + REPO_IP + ":8080/");
        b.expect("client", "^Added a \\(http://10\\.0\\.7\\.1:8080/\\): 1 packages", "index fetched", "^pkg: |not fetched");
        b.expect("client", PROMPT, "shell");
        b.send("client", "pkg search hello");
        b.expect("client", "^dev\\.ecm\\.hello 1\\.0\\.0  Hello  \\[a, unsigned\\]", "found in the repository");
        b.expect("client", "asks for: network", "its permissions are listed");
        b.expect("client", PROMPT, "shell");
        b.send("client", "pkg install " + APP);
        b.expect("client", "^Installed Hello 1\\.0\\.0 \\(dev\\.ecm\\.hello\\) - unverified publisher", "installed over the network", "^pkg: ");
        b.expect("repo", "GET /dev\\.ecm\\.hello-1\\.0\\.0\\.ecmpkg", "the repository served the package");
        b.expect("client", PROMPT, "shell");
        b.send("client", "pkg list");
        b.expect("client", "^dev\\.ecm\\.hello 1\\.0\\.0  Hello$", "listed as installed");
        b.expect("client", PROMPT, "shell");

        b.note("client: run it - nobody can be asked in console mode, so network is denied");
        b.send("client", "pkg run " + APP);
        b.expect("client", "^hello from dev\\.ecm\\.hello", "the app runs");
        b.expect("client", "^network: denied", "its connection is refused", "^network: ok");
        b.expect("client", "^pkg: dev\\.ecm\\.hello was denied network", "pkg names the denied permission");
        b.expect("client", PROMPT, "shell");

        b.note("client: grant network, run again");
        b.send("client", "pkg perm grant " + APP + " network");
        b.expect("client", "^dev\\.ecm\\.hello network allow", "granted");
        b.expect("client", PROMPT, "shell");
        b.send("client", "pkg run " + APP);
        b.expect("client", "^network: ok", "the granted app connects", "^network: denied");
        b.expect("client", PROMPT, "shell");
        return b.build();
    }

    // ------------------------------------------------------------------ app_store

    static com.example.evanscomputermod.computer.ComputerInstance computer(ScenarioRun r, String host) {
        var be = r.terminal(host);
        return be == null ? null : be.getComputer();
    }

    static com.example.evanscomputermod.computer.ui.Compositor comp(ScenarioRun r) {
        var c = computer(r, "client");
        return c == null || c.getUiManager() == null ? null : c.getUiManager().compositor();
    }

    /** Null once a window titled {@code title} is drawn (placed and mapped). */
    static String windowShown(ScenarioRun r, String title) {
        var c = comp(r);
        if (c == null) return "no GUI";
        return c.windows().stream().anyMatch(w -> w.title().equals(title) && w.mapped() && w.placed()) ? null : "no " + title + " window yet";
    }

    /** Click the text {@code text} in window {@code title} as the player would (null once clicked). */
    static String clickText(ScenarioRun r, String title, String text) {
        var c = comp(r);
        if (c == null) return "no GUI";
        int[] at = c.findText(title, text);
        if (at == null) return "\"" + text + "\" not shown in " + title + " yet";
        var ui = computer(r, "client").getUiManager();
        ui.pointer(1, at[0], at[1], 0, -1, 0);
        ui.pointer(2, at[0], at[1], 1, 0, 0);
        ui.pointer(3, at[0], at[1], 0, 0, 0);
        return null;
    }

    static final String NETCHECK = "dev.ecm.netcheck";

    /**
     * {@code app_store}: computer {@code repo} writes the sample repository
     * ({@code store --sample-repo}: Notes, the Visual Programmer's Counter and
     * Net Check) and serves it with {@code pkg serve}. Computer {@code client}
     * adds it, starts the GUI and opens the App Store (a system app, launched
     * through the app manager), opens Net Check's page, installs it with the
     * Install button (a download over the cable, checked against the index)
     * and opens it: its window appears. Net Check asks for {@code network};
     * the permission dialog is wired to the app manager in the integration
     * step, so here the request is recorded as a denial (console-mode
     * policy) and the app says so in its window. Controls: the store refuses to
     * start without the GUI; before the install the app manager doesn't know
     * Net Check.
     */
    static Scenario appStore() {
        var b = Scenario.builder("app_store",
                        "computer A serves the sample repository; computer B opens the App Store in the GUI, installs Net Check"
                                + " with its Install button and opens it (window); its network request is recorded")
                .asPlayer()
                .timeLimit(58_000);
        b.host("repo", new BlockPos(0, 1, 0), REPO_IP + "/24");
        b.host("client", new BlockPos(4, 1, 0), CLIENT_IP + "/24");
        b.link("lan", "repo", Direction.DOWN, "client", Direction.DOWN,
                Scenario.Path.from(new BlockPos(0, 0, 0)).go(Direction.EAST, 4));
        b.note("Setup (done for you): two computers, a cable between their bottom faces.");
        b.configureHosts();

        b.note("repo: write the sample repository and serve it");
        b.send("repo", "store --sample-repo /srv/repo");
        b.expect("repo", "^store: wrote the sample repository \\(3 apps\\)", "three sample packages written", "^store: (?!wrote)");
        b.expect("repo", PROMPT, "shell");
        b.send("repo", "pkg serve /srv/repo 8080 &");
        b.expect("repo", "Serving 3 packages from /srv/repo on port 8080", "repository online", "^pkg: ");

        b.note("client: the App Store is a system app; it needs the GUI (control)");
        b.send("client", "pkg list");
        b.expect("client", "^dev\\.ecm\\.store 1\\.0\\.0  App Store$", "the App Store is installed on every computer");
        b.expect("client", PROMPT, "shell");
        b.send("client", "store");
        b.expect("client", "needs the GUI", "in the console the store says to start the GUI");
        b.expect("client", PROMPT, "shell");
        b.send("client", "pkg repo add samples http://" + REPO_IP + ":8080/");
        b.expect("client", "^Added samples \\(http://10\\.0\\.7\\.1:8080/\\): 3 packages", "the repository's index fetched", "^pkg: |not fetched");
        b.expect("client", PROMPT, "shell");
        b.await(r -> computer(r, "client").getAppManager().manifest(NETCHECK) == null ? null : "already installed",
                "control: the app manager doesn't know Net Check yet", 1_000);

        b.note("client: start the GUI, then (Ctrl+Alt+Backspace to type) open the App Store through the app manager");
        b.send("client", "gui");
        b.expect("client", "^gui: started", "the compositor and the test shell start", "(?i)unknown command|no GUI device");
        b.await(r -> comp(r) != null && comp(r).shellConnected() ? null : "waiting for the shell", "the shell connected", 15_000);
        b.mutate(r -> {
            var c = computer(r, "client");
            c.handleKeyEvent(0x2A, com.example.evanscomputermod.computer.ui.UiManager.SWITCH_MODS, true, false);
            c.handleKeyEvent(0x2A, com.example.evanscomputermod.computer.ui.UiManager.SWITCH_MODS, false, false);
        }, "press Ctrl+Alt+Backspace (console)");
        b.await(r -> computer(r, "client").getUiManager().guiVisible() ? "GUI still shown" : null, "the console is shown", 5_000);
        b.send("client", "pkg run dev.ecm.store &");
        b.expect("client", "store: open", "the App Store runs as an app", "^pkg: ");
        b.await(r -> windowShown(r, "App Store"), "the App Store's window is drawn", 20_000);
        b.mutate(r -> {
            var c = computer(r, "client");
            c.handleKeyEvent(0x2A, com.example.evanscomputermod.computer.ui.UiManager.SWITCH_MODS, true, false);
            c.handleKeyEvent(0x2A, com.example.evanscomputermod.computer.ui.UiManager.SWITCH_MODS, false, false);
        }, "press Ctrl+Alt+Backspace (GUI)");
        b.await(r -> computer(r, "client").getUiManager().guiVisible() ? null : "console still shown", "the GUI is shown", 5_000);

        b.note("client: in the store, open Net Check's page and press Install");
        b.await(r -> clickText(r, "App Store", "Net Check"), "click Net Check in the catalog", 15_000);
        b.await(r -> comp(r).findText("App Store", "Use the network") != null ? null : "page not shown",
                "its page lists \"Use the network\" (the permission in plain words) and the publisher badge", 10_000);
        b.await(r -> clickText(r, "App Store", "Install"), "press Install", 5_000);
        b.expect("repo", "GET /dev\\.ecm\\.netcheck-1\\.0\\.0\\.ecmpkg", "the repository served the package");
        b.await(r -> computer(r, "client").getAppManager().manifest(NETCHECK) != null ? null : "not installed yet",
                "Net Check is installed and the app manager reindexed", 20_000);
        b.await(r -> comp(r).findText("App Store", "Installed Net Check 1.0.0 (unverified publisher)") != null ? null : "no message yet",
                "the store says it installed it (unverified publisher: unsigned)", 10_000);

        b.note("client: press Open - Net Check's window appears; its network request goes to the permission path");
        b.await(r -> clickText(r, "App Store", "Open"), "press Open", 10_000);
        b.await(r -> windowShown(r, "Net Check"), "Net Check's window is drawn", 30_000);
        b.await(r -> {
            var apps = computer(r, "client").getAppManager();
            var inst = apps.running().stream().filter(i -> i.appId().equals(NETCHECK)).findFirst().orElse(null);
            if (inst == null) return "Net Check not running";
            // No prompter is wired yet (integration step): the request is decided by the console-mode policy.
            return inst.denials().contains("network") ? null : "no network request yet";
        }, "Net Check asked for network (denied: the dialog is wired in the integration step)", 20_000);
        b.await(r -> comp(r).findText("Net Check", "Network: denied or unreachable") != null ? null : "no result shown",
                "Net Check shows the result in its window", 10_000);
        return b.build();
    }

    // After the string arrays above: static initialisers run in source order.
    static {
        Scenario s = appRepo();
        ALL.put(s.name, s);
        Scenario st = appStore();
        ALL.put(st.name, st);
    }

    private AppScenarios() {}
}
//?}
