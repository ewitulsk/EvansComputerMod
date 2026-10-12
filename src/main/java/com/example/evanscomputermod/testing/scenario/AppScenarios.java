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

    // After the string arrays above: static initialisers run in source order.
    static {
        Scenario s = appRepo();
        ALL.put(s.name, s);
    }

    private AppScenarios() {}
}
//?}
