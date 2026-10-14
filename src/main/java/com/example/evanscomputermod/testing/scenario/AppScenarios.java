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
        r.click("client", at[0], at[1]);
        return null;
    }

    /** Click the text {@code text} on the desktop's layer {@code layer} ("permission", "taskbar") or a popup ("popup"); null once clicked. */
    static String clickOn(ScenarioRun r, String layer, String text) {
        var c = comp(r);
        if (c == null) return "no GUI";
        int[] at = layer.equals("popup") ? c.locatePopupText(text, false) : c.locateLayerText(layer, text, false);
        if (at == null) return "\"" + text + "\" not shown on " + layer + " yet";
        r.click("client", at[0], at[1]);
        return null;
    }

    /** Ctrl+Alt+Backspace on the client's keyboard (console and GUI switch). */
    static void chord(ScenarioRun r) {
        r.pressKey("client", ScenarioPlayer.HID_BACKSPACE, com.example.evanscomputermod.computer.ui.UiManager.SWITCH_MODS, null);
    }

    /** The running Net Check (the last one launched), or null. */
    static com.example.evanscomputermod.computer.apps.AppInstance netCheck(ScenarioRun r) {
        com.example.evanscomputermod.computer.apps.AppInstance last = null;
        for (var i : computer(r, "client").getAppManager().running()) if (i.appId().equals(NETCHECK)) last = i;
        return last;
    }

    static final String NETCHECK = "dev.ecm.netcheck";
    static final String NETCHECK_ASKS = "Allow Net Check to use network?";

    /**
     * {@code app_store}: computer {@code repo} writes the sample repository
     * ({@code store --sample-repo}: Notes, the Visual Programmer's Counter and
     * Net Check) and serves it with {@code pkg serve}. Computer {@code client}
     * adds it, starts the GUI and opens the App Store (a system app, launched
     * through the app manager), opens Net Check's page, installs it with the
     * Install button (a download over the cable, checked against the index)
     * and opens it: its window appears. Net Check asks for {@code network}:
     * the desktop's permission dialog names it; Deny (control) makes its
     * connection fail. Closed with Alt+F4 and opened again from the desktop's
     * launcher (an installed app: launched through the app manager, so its
     * window carries its app id), it asks again; Allow, and it reaches the
     * repository. Controls: the store refuses to start without the GUI; before
     * the install the app manager doesn't know Net Check; the denied run
     * never connects.
     */
    static Scenario appStore() {
        var b = Scenario.builder("app_store",
                        "computer A serves the sample repository; computer B opens the App Store in the GUI, installs Net Check"
                                + " with its Install button and opens it; the desktop's permission dialog asks: Deny fails, Allow (after"
                                + " a relaunch from the launcher) connects")
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
        b.mutate(r -> chord(r), "press Ctrl+Alt+Backspace (console)");
        b.await(r -> computer(r, "client").getUiManager().guiVisible() ? "GUI still shown" : null, "the console is shown", 5_000);
        b.send("client", "pkg run dev.ecm.store &");
        b.expect("client", "store: open", "the App Store runs as an app", "^pkg: ");
        b.await(r -> windowShown(r, "App Store"), "the App Store's window is drawn", 20_000);
        b.mutate(r -> chord(r), "press Ctrl+Alt+Backspace (GUI)");
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

        b.note("client: press Open - Net Check asks for the network as it starts: the desktop's dialog (its window follows the answer)");
        b.await(r -> clickText(r, "App Store", "Open"), "press Open", 10_000);
        b.await(r -> comp(r).layerTexts("permission").contains(NETCHECK_ASKS) ? null : "no dialog yet",
                "the permission dialog asks \"" + NETCHECK_ASKS + "\" (the app's name from its app.toml)", 30_000);
        b.note("client: Deny (control) - the connection fails");
        b.await(r -> clickOn(r, "permission", "Deny"), "click Deny", 5_000);
        b.await(r -> windowShown(r, "Net Check"), "Net Check's window is drawn", 20_000);
        b.await(r -> comp(r).findText("Net Check", "Network: denied or unreachable") != null ? null : "no result shown",
                "Net Check says denied in its window", 15_000);
        b.await(r -> netCheck(r) != null && netCheck(r).denials().contains("network") ? null : "no denial recorded",
                "the app manager recorded the denial (not remembered)", 5_000);
        b.await(r -> computer(r, "client").getAppManager().permissions().decision(NETCHECK, "network") == null ? null : "remembered",
                "Remember my choice was off: nothing remembered", 1_000);
        b.note("client: close Net Check (click its window, Alt+F4) and open it again from the desktop's launcher");
        b.await(r -> clickText(r, "Net Check", "Net Check"), "click Net Check's window", 5_000);
        b.mutate(r -> r.pressKey("client", 0x3D, com.example.evanscomputermod.computer.ui.proto.UiProto.Mods.ALT, null), "press Alt+F4");
        b.await(r -> netCheck(r) == null ? null : "still running", "Net Check quit", 10_000);
        b.await(r -> clickOn(r, "taskbar", "Apps"), "click Apps on the taskbar", 5_000);
        b.await(r -> clickOn(r, "popup", "Net Check"), "click Net Check in the launcher (listed since its install)", 5_000);
        b.await(r -> comp(r).layerTexts("permission").contains(NETCHECK_ASKS) ? null : "no dialog yet", "it asks again (new run)", 30_000);
        b.await(r -> clickOn(r, "permission", "Allow"), "click Allow", 5_000);
        b.await(r -> {
            var c = comp(r);
            return c.windows().stream().anyMatch(w -> w.title().equals("Net Check") && w.mapped() && w.appId().equals(NETCHECK))
                    ? null : "no Net Check window with its app id yet";
        }, "Net Check's window again, its app id from the app manager (launched as the app, not as python)", 20_000);
        b.await(r -> comp(r).findText("Net Check", "Network: allowed (reached " + REPO_IP + ":8080)") != null ? null : "not connected yet",
                "Net Check reaches the repository", 15_000);
        return b.build();
    }

    // ------------------------------------------------------------------ files_open_with

    static final String EDITOR_APP = com.example.evanscomputermod.computer.apps.AppManager.TEXT_EDITOR;

    /** The Text Editor app's running instance, or null. */
    static com.example.evanscomputermod.computer.apps.AppInstance editorApp(ScenarioRun r) {
        var c = GuiScenarios.computer(r);
        if (c == null) return null;
        for (var i : c.getAppManager().running()) if (i.appId().equals(EDITOR_APP)) return i;
        return null;
    }

    /** Double-click a drawn text in an app's window, as a player does (two clicks within the double-click time). */
    static void doubleClick(ScenarioRun r, String appId, String text) {
        GuiScenarios.clickText(r, appId, text);
        GuiScenarios.clickText(r, appId, text);
    }

    /**
     * {@code files_open_with} (ROADMAP P4 "Double-clicking a .py file in Files
     * opens the Editor, which can read only that file"): one computer with
     * {@code /home/hello.py} and {@code /home/secret.py}. The player starts the
     * GUI, opens Files from the launcher, double-clicks {@code home}, then
     * {@code hello.py}: Files sends {@code Open}, the app manager opens it with
     * the Text Editor app ({@code dev.ecm.editor}, a system app with no
     * permissions) and a one-file grant, and the Editor shows the file. With
     * Ctrl+O the player then asks the same Editor for {@code secret.py}: it
     * can't see it (a forbidden read looks like a missing file), so it offers a
     * new file and never shows the secret. Controls: Files (a built-in program,
     * not an app) lists both files; the app manager allows the Editor the
     * opened file only.
     */
    static Scenario filesOpenWith() {
        var b = Scenario.builder("files_open_with",
                        "double-clicking a .py file in Files opens it in the Text Editor app, which can read only that file")
                .asPlayer()
                .timeLimit(58_000);
        b.host("pc", new BlockPos(0, 1, 0), "-");
        b.note("Write /home/hello.py and /home/secret.py from Python");
        Map<String, String> home = new LinkedHashMap<>();
        home.put("/home/hello.py", "print('hello from the file')");
        home.put("/home/secret.py", "top-secret = 42");
        PlayerKit.typeFiles(b, Map.of("pc", home));
        b.send("pc", "pkg list");
        b.expect("pc", "^dev\\.ecm\\.editor 1\\.0\\.0  Text Editor$", "the Text Editor app is a system app on every computer");
        b.expect("pc", PROMPT, "shell");
        b.send("pc", "gui");
        b.expect("pc", "^gui: started", "the GUI starts with the desktop", "(?i)unknown command|no GUI device");
        b.await(r -> GuiScenarios.comp(r) != null && GuiScenarios.comp(r).shellConnected() ? null : "no desktop yet", "the desktop is connected", 15_000);
        b.note("Files from the launcher; double-click home, then hello.py");
        GuiScenarios.launch(b, "Files", "files")
                .await(r -> GuiScenarios.visible(r, "files", "home"), "Files lists the disk", 10_000)
                .mutate(r -> doubleClick(r, "files", "home"), "double-click home")
                .await(r -> GuiScenarios.visible(r, "files", "hello.py"), "Files shows /home", 10_000)
                .await(r -> GuiScenarios.visible(r, "files", "secret.py"), "control: Files (not an app) lists secret.py too", 2_000)
                .mutate(r -> doubleClick(r, "files", "hello.py"), "double-click hello.py")
                .await(r -> GuiScenarios.shows(r, EDITOR_APP, "print('hello from the file')"),
                        "the Text Editor app opens hello.py (Open -> the app manager's default for .py)", 30_000)
                .await(r -> {
                    var e = editorApp(r);
                    if (e == null) return "the editor isn't an app";
                    var apps = GuiScenarios.computer(r).getAppManager();
                    if (!apps.checkPath(e.pid(), "/home/hello.py", true)) return "no grant for hello.py";
                    return apps.checkPath(e.pid(), "/home/secret.py", false) ? "it may read secret.py!" : null;
                }, "the Editor runs as dev.ecm.editor with a one-file grant: hello.py yes, secret.py no", 2_000)
                .note("Ctrl+O in the Editor, /home/secret.py, Enter: it can't see the file")
                .mutate(r -> GuiScenarios.key(r, 0x12, com.example.evanscomputermod.computer.ui.proto.UiProto.Mods.CTRL), "press Ctrl+O")
                .waitMs(300, "the path field appears")
                .mutate(r -> GuiScenarios.key(r, 0x04, com.example.evanscomputermod.computer.ui.proto.UiProto.Mods.CTRL), "Ctrl+A in the path field")
                .mutate(r -> GuiScenarios.type(r, "/home/secret.py"), "type /home/secret.py")
                .mutate(GuiScenarios::enter, "Enter")
                .await(r -> GuiScenarios.shows(r, EDITOR_APP, "New file /home/secret.py"),
                        "the Editor can't see secret.py (a forbidden read looks like a missing file)", 10_000)
                .await(r -> GuiScenarios.shows(r, EDITOR_APP, "top-secret") == null ? "it shows the secret!" : null,
                        "the secret is never shown", 1_000)
                .mutate(r -> GuiScenarios.snapshot(r, "files_open_with"), "snapshot: Files and the Editor");
        return b.build();
    }

    // ------------------------------------------------------------------ chat_app

    static final String CHAT = "dev.ecm.chat", CHAT_SERVER = "10.0.8.1";
    static final String CHAT_HELLO = "hello from my desk", CHAT_REPLY = "hi alex, welcome", CHAT_LATER = "are you still there?";

    /** Click "Repositories" (medium and wide windows) or "Repos" (compact) in the App Store. */
    static String clickRepos(ScenarioRun r) {
        return clickText(r, "App Store", "Repositories") == null ? null : clickText(r, "App Store", "Repos");
    }

    /** Null once Chat's window shows a text containing {@code text}. */
    static String chatShows(ScenarioRun r, String text) {
        var c = comp(r);
        if (c == null) return "no GUI";
        for (var w : c.windows()) {
            if (!w.appId().equals(CHAT) || !w.mapped()) continue;
            var texts = c.textsOf(w.id());
            return texts.stream().anyMatch(t -> t.contains(text)) ? null : "Chat shows " + texts;
        }
        return "no Chat window";
    }

    static boolean chatFocused(ScenarioRun r) {
        return comp(r).windows().stream().anyMatch(w -> w.appId().equals(CHAT) && w.focused());
    }

    /** Type on the client's keyboard (into the focused field): the player's key packets. */
    static void typeOn(Scenario.Builder b, String text) {
        b.mutate(r -> r.typeText("client", text), "type '" + text + "'");
    }

    /**
     * {@code chat_app} (docs/gui/APPS.md "Chat"): computer {@code server} gets an address and
     * runs {@code dhcpd} with no config (it writes a default one), {@code chatd}, builds the
     * Chat package itself ({@code chatapp --source}, {@code pkg build}) and serves it
     * ({@code pkg serve}). Computer {@code client}, cabled to it, gets a lease with
     * {@code dhclient}, starts the GUI, opens the App Store from the launcher, adds the
     * server's repository on the Repositories page, installs Chat and opens it; types the
     * server's address and a nickname, Connect; the desktop's dialog asks for the network:
     * Allow. The server joins with the {@code chat} command and sees the client's message; its
     * reply shows in Chat. Then Files is opened (Chat loses focus) and a message from the server
     * makes Chat post a notification (after the desktop asks for notifications: Allow).
     * Controls: no repository in the store at first; Chat isn't installed before Install; no
     * notification while Chat has focus.
     */
    static Scenario chatApp() {
        var b = Scenario.builder("chat_app",
                        "a server runs chatd and serves the Chat app it packaged; a cabled client gets a DHCP lease, installs Chat from"
                                + " the App Store and talks to the server's chat command; a message while Chat is in the background"
                                + " posts a notification")
                .asPlayer()
                .timeLimit(58_000);
        b.host("server", new BlockPos(0, 1, 0), CHAT_SERVER + "/24");
        b.host("client", new BlockPos(4, 1, 0), null);
        b.link("lan", "server", Direction.DOWN, "client", Direction.DOWN,
                Scenario.Path.from(new BlockPos(0, 0, 0)).go(Direction.EAST, 4));
        b.note("Setup (done for you): two computers, a cable between their bottom faces.");
        b.send("server", "ifconfig eth0 " + CHAT_SERVER + "/24");
        b.expect("server", "eth0: inet 10\\.0\\.8\\.1/24", "server has " + CHAT_SERVER);

        b.note("server: DHCP with no config (dhcpd writes a default one), the chat server, the Chat package, a repository");
        b.send("server", "dhcpd &");
        b.expect("server", "no /etc/dhcpd\\.conf; wrote a default config for eth0 there", "dhcpd writes /etc/dhcpd.conf");
        b.expect("server", "pool eth0 10\\.0\\.8\\.100 10\\.0\\.8\\.199 prefix 24 router 10\\.0\\.8\\.1 lease 3600", "the pool: .100-.199, itself as router");
        b.expect("server", "serving eth0 10\\.0\\.8\\.100-10\\.0\\.8\\.199/24 as 10\\.0\\.8\\.1", "dhcpd serving");
        b.send("server", "chatd &");
        b.expect("server", "chatd listening on port 7777", "chatd listening");
        b.send("server", "chatapp --source /src/chat");
        b.expect("server", "chatapp: wrote Chat \\(dev\\.ecm\\.chat\\) 1\\.0\\.0 to /src/chat", "Chat's package folder: app.toml, icon.png, main.wasm",
                "^chatapp: can't");
        b.expect("server", PROMPT, "shell");
        b.send("server", "pkg build /src/chat /srv/repo");
        b.expect("server", "^Built /srv/repo/dev\\.ecm\\.chat-1\\.0\\.0\\.ecmpkg \\(3 files", "pkg build packs it", "^pkg: ");
        b.expect("server", PROMPT, "shell");
        b.send("server", "pkg serve /srv/repo 8080 &");
        b.expect("server", "Serving 1 packages from /srv/repo on port 8080", "the repository is online", "^pkg: ");

        b.note("client: a lease from the server's default pool");
        b.send("client", "dhclient eth0");
        b.expect("client", "bound to 10\\.0\\.8\\.100/24", "the client leased 10.0.8.100", "no lease");
        b.expect("client", PROMPT, "shell");
        b.await(r -> computer(r, "client").getAppManager().manifest(CHAT) == null ? null : "already installed",
                "control: Chat isn't installed", 1_000);

        b.note("client: the GUI; the App Store from the launcher; Repositories: the server's address, Add");
        b.send("client", "gui");
        b.expect("client", "^gui: started", "the desktop starts", "(?i)unknown command|no GUI device");
        b.await(r -> comp(r) != null && comp(r).shellConnected() ? null : "waiting for the shell", "the desktop is up", 15_000);
        b.await(r -> clickOn(r, "taskbar", "Apps"), "click Apps on the taskbar", 10_000);
        b.await(r -> clickOn(r, "popup", "App Store"), "click App Store in the launcher", 5_000);
        b.await(r -> windowShown(r, "App Store"), "the App Store's window", 20_000);
        b.await(r -> comp(r).findText("App Store", "No apps yet. Add a repository under Repos.") != null ? null : "store not loaded",
                "control: no repository, no apps", 15_000);
        b.await(AppScenarios::clickRepos, "click Repositories", 5_000);
        b.await(r -> clickText(r, "App Store", "http://10.0.30.1:8080/"), "click the address field", 5_000);
        typeOn(b, CHAT_SERVER + ":8080");
        b.await(r -> clickText(r, "App Store", "Add"), "click Add", 5_000);
        b.await(r -> comp(r).findText("App Store", "Chat") != null ? null : "Chat not listed",
                "the repository is added (10-0-8-1) and lists Chat", 20_000);
        b.await(r -> clickText(r, "App Store", "Install"), "click Install", 5_000);
        b.expect("server", "GET /dev\\.ecm\\.chat-1\\.0\\.0\\.ecmpkg", "the repository served the package");
        b.await(r -> computer(r, "client").getAppManager().manifest(CHAT) != null ? null : "not installed yet", "Chat is installed", 20_000);
        b.await(r -> clickText(r, "App Store", "Open"), "click Open", 15_000);
        b.await(r -> windowShown(r, "Chat"), "Chat's window", 20_000);

        b.note("Chat: the server's address and a nickname, Connect; the desktop asks for the network: Allow");
        b.await(r -> clickText(r, "Chat", "10.0.30.1"), "click the server field", 5_000);
        typeOn(b, CHAT_SERVER);
        b.await(r -> clickText(r, "Chat", "Nickname (optional)"), "click the nickname field", 5_000);
        typeOn(b, "alex");
        b.await(r -> clickText(r, "Chat", "Connect"), "click Connect", 5_000);
        b.await(r -> comp(r).layerTexts("permission").contains("Allow Chat to use network?") ? null : "no dialog: " + comp(r).layerTexts("permission"),
                "the dialog asks \"Allow Chat to use network?\"", 20_000);
        b.await(r -> clickOn(r, "permission", "Allow"), "click Allow", 5_000);
        b.await(r -> chatShows(r, "as alex"), "Chat is connected as alex", 20_000);

        b.note("server: join with the chat command; the client's message arrives, the reply shows in Chat");
        b.send("server", "chat " + CHAT_SERVER + " --nick base");
        b.expect("server", "Joined Tech Village chat as base", "the server's chat command joins");
        b.await(r -> clickText(r, "Chat", "Message Tech Village chat"), "click the message box", 10_000);
        typeOn(b, CHAT_HELLO);
        b.await(r -> clickText(r, "Chat", "Send"), "click Send", 5_000);
        b.expect("server", "<alex> " + CHAT_HELLO, "the server's chat shows the client's message");
        b.send("server", CHAT_REPLY);
        b.await(r -> chatShows(r, CHAT_REPLY), "the reply shows in Chat", 15_000);
        b.await(r -> comp(r).notifications().stream().noneMatch(n -> n.appId().equals(CHAT)) ? null : "a notification while on screen",
                "control: no notification while Chat has focus", 1_000);

        b.note("Chat in the background: Files from the launcher takes the focus; the server writes again");
        b.await(r -> clickOn(r, "taskbar", "Apps"), "click Apps on the taskbar", 5_000);
        b.await(r -> clickOn(r, "popup", "Files"), "click Files in the launcher", 5_000);
        b.await(r -> windowShown(r, "Files") == null && !chatFocused(r) ? null : "Chat still focused", "Files has the focus", 15_000);
        b.send("server", CHAT_LATER);
        b.await(r -> comp(r).layerTexts("permission").contains("Allow Chat to use notifications?") ? null
                : "no dialog: " + comp(r).layerTexts("permission"), "Chat's first notification: the desktop asks for notifications", 15_000);
        b.await(r -> clickOn(r, "permission", "Allow"), "click Allow", 5_000);
        b.await(r -> comp(r).notifications().stream().anyMatch(n -> n.appId().equals(CHAT) && n.body().contains(CHAT_LATER)
                        && n.title().equals("base in Tech Village chat")) ? null : "notifications " + comp(r).notifications(),
                "Chat posts \"base in Tech Village chat: " + CHAT_LATER + "\"", 15_000);
        b.await(r -> chatShows(r, CHAT_LATER), "the message is in Chat too", 5_000);
        return b.build();
    }

    // After the string arrays above: static initialisers run in source order.
    static {
        Scenario s = appRepo();
        ALL.put(s.name, s);
        Scenario st = appStore();
        ALL.put(st.name, st);
        Scenario fo = filesOpenWith();
        ALL.put(fo.name, fo);
        Scenario ca = chatApp();
        ALL.put(ca.name, ca);
    }

    private AppScenarios() {}
}
//?}
