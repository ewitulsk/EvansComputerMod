package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.EcmConfig;
import com.example.evanscomputermod.api.repo.RepoLinks;
import com.example.evanscomputermod.computer.repo.PlayerPrompts;
import com.example.evanscomputermod.computer.repo.RepoServers;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * The official package repository, against its real backend (docs/REPO.md):
 * the four ways a server lets computers reach it, and accounts.
 *
 * <ul>
 * <li>{@code repo_open}: every computer, with no network at all;
 * <li>{@code repo_internet}: a computer cabled to an Internet Gateway;
 * <li>{@code repo_datacenter}: a repository server mirrors all of it for the
 *     computers cabled to it;
 * <li>{@code repo_curated}: the same, offering only a chosen package;
 * <li>{@code repo_account}: sign in with an API key, publish (confirmed in
 *     chat), install, like, favorite, yank, sign out;
 * <li>{@code repo_login}: sign in with a code approved in the browser.
 * </ul>
 *
 * The backend is outside the game, as the real one is. These scenarios use
 * the one {@code scripts/repo-backend-local.sh up} starts on this machine
 * (the real services with fixture packages {@code com.ecmtest.*}), read from
 * the environment it writes; see docs/SCENARIO_EXCEPTIONS.md.
 */
public final class RepoBackendScenarios {

    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static final String PROMPT = PlayerKit.PROMPT;

    static String env(String name) {
        String v = System.getenv(name);
        return v == null ? "" : v.trim();
    }

    /** The repository's address, as the server would be configured ({@code repo.url}). */
    static String url() {
        return env("ECM_TEST_REPO_URL");
    }

    static String key() {
        return env("ECM_TEST_REPO_KEY");
    }

    static String host() {
        return url().replaceFirst("^https?://", "").replaceFirst(":.*$", "");
    }

    /**
     * The server's repository settings for a scenario: set when the layout is
     * built (before its computers boot and read them), back to the config when
     * it is cleared. An operator does this in config/evanscomputermod-common.toml.
     */
    static Scenario.Decor settings(String access) {
        return new Scenario.Decor() {
            public List<BlockPos> footprint() {
                return List.of();
            }

            public void build(ScenarioRun r) {
                EcmConfig.overrideRepo(access, url(), key());
            }

            public void clear(ScenarioRun r) {
                EcmConfig.overrideRepo(null, null, null);
                for (String n : r.scenario().nodes.keySet()) {
                    var id = r.terminal(n) == null ? null : r.computerId(n);
                    if (id != null) RepoServers.revoke(r.level().getServer(), id);
                }
            }
        };
    }

    /** Null when the backend's settings are known and (for {@code link}) the server can reach it itself. */
    static String ready(ScenarioRun r, boolean link) {
        if (url().isEmpty() || key().length() != 64) {
            r.fail("this scenario needs the repository's backend: run scripts/repo-backend-local.sh up, then start the server with the"
                    + " settings it prints (source runs/repo-backend/env): ECM_TEST_REPO_URL and ECM_TEST_REPO_KEY are not set");
            return "no backend settings";
        }
        if (link && RepoLinks.get() == null) {
            r.fail("the server's own link to the repository needs the evanscomputermod-internet sidecar in the server's mods folder");
            return "no repository link";
        }
        return null;
    }

    static void run(Scenario.Builder b, String node, String command, String re, String what) {
        RepoScenarios.run(b, node, command, re, what);
    }

    static void run(Scenario.Builder b, String node, String command, String re, String what, String failRe) {
        RepoScenarios.run(b, node, command, re, what, failRe);
    }

    static String q(String s) {
        return Pattern.quote(s);
    }

    /** After a restart: the computer is back at its prompt (it reads the server's repository settings as it boots). */
    static void restart(Scenario.Builder b, String node) {
        PlayerKit.powerCycle(b, node);
        b.await(r -> Pattern.compile("(?m)^/\\S* >\\s*$").matcher(r.screen(node)).find() ? null : "not booted yet", node + ": booted again", 20_000);
    }

    // ------------------------------------------------------------------ repo_open

    /**
     * {@code repo_open}: the server offers the repository to every computer
     * ({@code repo.access = open}). One computer with no cable at all finds
     * the official repository already in {@code /etc/pkg/repos}
     * ({@code host://}: the server fetches for it), {@code pkg online} says it
     * needs no network, and it installs an app with the library it needs,
     * verified against the pinned key; the community channel is there too.
     * Control: the operator switches to {@code repo.access = internet} and the
     * computer restarts: now the repository is an address on the network,
     * {@code pkg online} says the computer has no address, and the server's link
     * refuses a computer that asks for it by hand.
     */
    static Scenario open() {
        var b = Scenario.builder("repo_open",
                        "repo.access = open: a computer with no cable installs from the official repository through the server;"
                                + " switched to internet, the same computer is told it has no network")
                .asPlayer()
                .timeLimit(58_000);
        b.host("pc", new BlockPos(0, 1, 0), null);
        b.decor(settings("open"));
        b.note("Setup (done for you): one computer, nothing plugged into it. The server's config has repo.access = open.");
        b.await(r -> ready(r, true), "the repository's backend is up and the server can reach it", 2_000);

        b.note("pc: the official repository is already set up, and needs no network");
        b.sendFn("pc", r -> "cat /etc/pkg/repos", "cat /etc/pkg/repos");
        b.await(r -> r.screen("pc").contains("official host://repo/r/stable/ " + key()) ? null : "no official line yet",
                "the server put the official repository in /etc/pkg/repos, key pinned", 10_000);
        b.expect("pc", "^community host://repo/r/community/ ", "and the community channel");
        b.expect("pc", PROMPT, "pc: shell");
        run(b, "pc", "pkg online official", "^official  host://repo/r/stable/  online \\(through the server; no network needed\\)", "pkg online: no network needed");
        b.send("pc", "pkg update");
        b.expect("pc", "^official: 4 packages", "the stable channel's index arrived", "^pkg: official");
        b.expect("pc", "^community: 5 packages", "and the community channel's");
        b.expect("pc", PROMPT, "pc: shell");
        run(b, "pc", "pkg search hello", "^com\\.ecmtest\\.hello 1\\.0\\.0  Hello  \\[official, signed\\]", "pkg search finds a package there");
        b.send("pc", "pkg install com.ecmtest.area");
        b.expect("pc", "^Installed com\\.ecmtest\\.geo 1\\.0\\.0 \\(needed by com\\.ecmtest\\.area\\)", "the library it needs came first", "^pkg: ");
        b.expect("pc", "^Installed Area 1\\.0\\.0 \\(com\\.ecmtest\\.area\\) - verified publisher", "installed, checked against the pinned key", "^pkg: ");
        b.expect("pc", PROMPT, "pc: shell");
        run(b, "pc", "lua /apps/com.ecmtest.area/main.lua", "^area is 42$", "the app runs with its library", "not found");
        run(b, "pc", "pkg search fresh", "^com\\.ecmtest\\.fresh 1\\.0\\.0  Fresh  \\[community, signed\\]", "a package only the community channel has");

        b.note("Control: the operator sets repo.access = internet; the computer is restarted and has no network");
        b.mutate(r -> EcmConfig.overrideRepo("internet", url(), key()), "server config: repo.access = internet");
        restart(b, "pc");
        b.sendFn("pc", r -> "cat /etc/pkg/repos", "cat /etc/pkg/repos");
        b.await(r -> r.screen("pc").contains("official " + url() + "/r/stable/ " + key()) ? null : "still the old line",
                "the repository is now an address on the network", 10_000);
        b.expect("pc", PROMPT, "pc: shell");
        run(b, "pc", "pkg online official", "this computer has no address", "control: pkg online names what is missing", "online \\(");
        run(b, "pc", "pkg repo add direct host://repo/r/stable/", "does not link to the package repository itself", "control: the server's link is not offered now",
                "packages$");
        return b.build();
    }

    // ------------------------------------------------------------------ repo_internet

    /**
     * {@code repo_internet}: {@code repo.access = internet}. A computer cabled
     * to an Internet Gateway finds the official repository in
     * {@code /etc/pkg/repos} as an address on the network. Before it has an
     * address {@code pkg online} says so; with an address and a route it
     * installs a package over its own network, through the gateway.
     * Control: the server's own link is not offered in this mode.
     */
    static Scenario internet() {
        var b = Scenario.builder("repo_internet",
                        "repo.access = internet: a computer cabled to an Internet Gateway installs from the official repository over its own"
                                + " network; without an address pkg online says what is missing")
                .asPlayer()
                .timeLimit(58_000);
        b.host("client", new BlockPos(0, 1, 0), null);
        List<BlockPos> cable = List.of(new BlockPos(0, 0, 0), new BlockPos(0, 0, 1));
        BlockPos gateway = new BlockPos(0, 0, 2);
        b.decor(settings("internet"));
        b.decor(new Scenario.Decor() {
            public List<BlockPos> footprint() {
                List<BlockPos> f = new java.util.ArrayList<>(cable);
                f.add(gateway);
                return f;
            }

            public void build(ScenarioRun r) {
                for (BlockPos p : cable) r.player().place(com.example.evanscomputermod.block.ModBlocks.NETWORK_CABLE.get(), r.abs(p), Direction.UP);
                r.player().place(com.example.evanscomputermod.block.ModBlocks.INTERNET_GATEWAY.get(), r.abs(gateway), Direction.SOUTH);
            }
        });
        b.note("Setup (done for you): a computer, a cable from its bottom face to an Internet Gateway. The server's config has repo.access = internet.");
        b.await(r -> {
            String e = ready(r, false);
            if (e != null) return e;
            var hub = com.example.evanscomputermod.computer.NetworkHub.getInstance();
            if (hub == null || !hub.uplinkOpen()) {
                r.fail("the Internet Gateway is offline: the server needs the evanscomputermod-internet sidecar");
                return "no uplink";
            }
            // The local backend is on this machine's LAN, which the gateway refuses unless the server allows it.
            if (!Boolean.getBoolean("evanscomputermod.internet.allowLan") && host().matches("(10|192\\.168|172\\.(1[6-9]|2\\d|3[01])|100\\.(6[4-9]|[7-9]\\d|1[01]\\d|12[0-7]))\\..*")) {
                r.fail("the backend at " + url() + " is on the LAN: start the server with -Devanscomputermod.internet.allowLan=true"
                        + " (JAVA_TOOL_OPTIONS) to let the gateway reach it");
                return "LAN blocked";
            }
            return null;
        }, "the backend is up and the Internet Gateway is online", 2_000);

        b.note("client: the official repository is an address on the network; control: no address yet");
        b.sendFn("client", r -> "cat /etc/pkg/repos", "cat /etc/pkg/repos");
        b.await(r -> r.screen("client").contains("official " + url() + "/r/stable/ " + key()) ? null : "no official line yet",
                "the server put the official repository's address in /etc/pkg/repos", 10_000);
        b.expect("client", PROMPT, "client: shell");
        run(b, "client", "pkg online official", "connected, but this computer has no address", "control: pkg online says an address is missing", "online$");

        b.note("client: an address on the gateway's network and a default route (the gateway serves no DHCP)");
        // A /30 holds just the gateway (10.0.0.1) and this computer, so every other address goes
        // through the gateway: the local backend may itself be at a 10.0.0.x address of the real LAN.
        run(b, "client", "ifconfig eth0 10.0.0.2/30", "eth0: inet 10\\.0\\.0\\.2/30", "the client has an address");
        run(b, "client", "ip route add default via 10.0.0.1 dev eth0", PROMPT, "and a default route to the gateway");
        b.sendFn("client", r -> "pkg online official", "pkg online official");
        b.await(r -> Pattern.compile("(?m)^official  " + q(url() + "/r/stable/") + "  online\\s*$").matcher(r.screen("client")).find() ? null : "not online yet",
                "pkg online: the repository answers through the gateway", 20_000);
        b.expect("client", PROMPT, "client: shell");
        b.send("client", "pkg install com.ecmtest.hello");
        b.expect("client", "^Installed Hello 1\\.0\\.0 \\(com\\.ecmtest\\.hello\\) - verified publisher", "installed over the computer's own network", "^pkg: ");
        b.expect("client", PROMPT, "client: shell");
        run(b, "client", "lua /apps/com.ecmtest.hello/main.lua", "^hello from the repository$", "the app runs");
        run(b, "client", "pkg repo add direct host://repo/r/stable/", "does not link to the package repository itself", "control: the server's own link is not offered",
                "packages$");
        return b.build();
    }

    // ------------------------------------------------------------------ repo_datacenter, repo_curated

    static final String DC_IP = "10.0.11.1", CLIENT_IP = "10.0.11.2";

    /**
     * {@code repo_datacenter} and {@code repo_curated}:
     * {@code repo.access = datacenter}. Computer {@code dc} is refused the
     * server's link until an operator names it a repository server
     * ({@code /ecm repo grant}); restarted, it has the official repository,
     * mirrors it ({@code pkg mirror}: all of it, or with {@code --only} one
     * app and what that needs) and offers the copy on the game's network
     * ({@code pkg serve}). Computer {@code client}, cabled to it, adds the
     * data center's repository with the official key pinned and installs:
     * the copy verifies as the original would. Controls: {@code dc} before
     * the grant; {@code client} asking for the server's link itself; and, in
     * {@code repo_curated}, a package the data center did not choose.
     */
    static Scenario datacenter(boolean curated) {
        String name = curated ? "repo_curated" : "repo_datacenter";
        var b = Scenario.builder(name,
                        "repo.access = datacenter: an operator names a repository server (/ecm repo grant); it mirrors "
                                + (curated ? "one chosen app" : "the official repository") + " (pkg mirror) and serves it; a cabled client installs"
                                + " from it; other computers are refused the server's link" + (curated ? " and are not offered what was not chosen" : ""))
                .asPlayer()
                .timeLimit(58_000);
        b.host("dc", new BlockPos(0, 1, 0), DC_IP + "/24");
        b.host("client", new BlockPos(4, 1, 0), CLIENT_IP + "/24");
        b.link("lan", "dc", Direction.DOWN, "client", Direction.DOWN, Scenario.Path.from(new BlockPos(0, 0, 0)).go(Direction.EAST, 4));
        b.decor(settings("datacenter"));
        b.note("Setup (done for you): two computers, a cable between their bottom faces. The server's config has repo.access = datacenter.");
        b.await(r -> ready(r, true), "the repository's backend is up and the server can reach it", 2_000);
        b.configureHosts();

        b.note("dc: control - not a repository server yet, so the server's link refuses it");
        b.sendFn("dc", r -> "pkg repo add up host://repo/r/stable/ --key " + key(), "pkg repo add up host://repo/r/stable/ --key <the repository's key>");
        b.expect("dc", "only lets its repository servers use that link", "control: refused before the grant", "packages$");
        b.expect("dc", PROMPT, "dc: shell");
        run(b, "dc", "pkg repo remove up", "^Removed up", "removed again");

        b.note("Operator: /ecm repo grant <the position of dc>; dc is restarted and has the official repository");
        b.mutate(r -> {
            BlockPos at = r.where("dc");
            List<String> said = r.player().command("/ecm repo grant " + at.getX() + " " + at.getY() + " " + at.getZ());
            if (said.stream().noneMatch(s -> s.contains("is now a repository server"))) r.fail("/ecm repo grant said: " + said);
        }, "/ecm repo grant <x y z of dc>");
        restart(b, "dc");
        run(b, "dc", "ifconfig eth0 " + DC_IP + "/24", "eth0: inet " + q(DC_IP + "/24"), "dc has its address again");
        b.sendFn("dc", r -> "cat /etc/pkg/repos", "cat /etc/pkg/repos");
        b.await(r -> r.screen("dc").contains("official host://repo/r/stable/ " + key()) ? null : "no official line yet",
                "a repository server is given the official repository (through the server)", 10_000);
        b.expect("dc", PROMPT, "dc: shell");

        b.note("dc: mirror " + (curated ? "one app (what it needs comes along)" : "the whole stable channel") + " and serve the copy");
        if (curated) {
            run(b, "dc", "pkg mirror official /srv/mirror --only com.ecmtest.area", "^2 copied, 0 already there\\.", "the app and its library were copied", "^pkg: ");
        } else {
            run(b, "dc", "pkg mirror official /srv/mirror", "^4 copied, 0 already there\\.", "the four packages were copied", "^pkg: ");
        }
        b.send("dc", "pkg serve /srv/mirror 8080 &");
        b.expect("dc", "Serving " + (curated ? 2 : 4) + " packages from /srv/mirror on port 8080\\.", "the data center offers its copy", "^pkg: ");

        b.note("client: the data center's repository, with the official key pinned");
        b.sendFn("client", r -> "pkg repo add dc http://" + DC_IP + ":8080/ --key " + key(), "pkg repo add dc http://" + DC_IP + ":8080/ --key <the repository's key>");
        b.expect("client", "^Added dc .*: " + (curated ? 2 : 4) + " packages", "the client sees what the data center offers", "^pkg: |not fetched");
        b.expect("client", PROMPT, "client: shell");
        b.send("client", "pkg install com.ecmtest.area");
        b.expect("client", "^Installed Area 1\\.0\\.0 \\(com\\.ecmtest\\.area\\) - verified publisher", "installed from the data center, verified against the official key", "^pkg: ");
        b.expect("client", PROMPT, "client: shell");
        b.expect("dc", "GET /com\\.ecmtest\\.area-1\\.0\\.0\\.ecmpkg", "the data center served it");
        run(b, "client", "lua /apps/com.ecmtest.area/main.lua", "^area is 42$", "the app runs with its library");
        if (curated) {
            run(b, "client", "pkg install com.ecmtest.hello", "^pkg: com\\.ecmtest\\.hello is not in any repository", "control: what the data center did not choose is not offered");
        } else {
            run(b, "client", "pkg install com.ecmtest.hello", "^Installed Hello 1\\.0\\.0 \\(com\\.ecmtest\\.hello\\) - verified publisher", "everything in stable is offered", "^pkg: ");
        }
        b.note("client: control - an ordinary computer asking for the server's link itself");
        run(b, "client", "pkg repo add direct host://repo/r/stable/", "only lets its repository servers use that link", "control: the client is refused the server's link",
                "packages$");
        return b.build();
    }

    // ------------------------------------------------------------------ accounts

    /** This run's package id: a fresh one, so the first publish makes it this account's. */
    static String myId(ScenarioRun r) {
        if (r.noted("id").isEmpty()) r.note("id", "com.ecmtest.r" + (System.currentTimeMillis() / 1000));
        return r.noted("id");
    }

    /** The open question the scenario's player was asked in chat (their Allow / Deny buttons). */
    static Integer asked(ScenarioRun r) {
        return PlayerPrompts.pendingFor(r.player().entity().getUUID());
    }

    /** {@code pkg publish} waits for the player: click Allow or Deny in chat (the button runs /ecmconfirm). */
    static void confirm(Scenario.Builder b, boolean allow) {
        b.await(r -> asked(r) != null && r.player().heard().stream().anyMatch(s -> s.contains("[Allow]") && s.contains("[Deny]")) ? null : "not asked yet",
                "the player is asked in chat: Publish ...? [Allow] [Deny]", 15_000);
        b.mutate(r -> r.player().command("/ecmconfirm " + asked(r) + (allow ? " allow" : " deny")), "click " + (allow ? "[Allow]" : "[Deny]") + " in chat");
    }

    /**
     * {@code repo_account}: on a server with {@code repo.access = open}, a
     * player writes a small app and publishes it under their account. Not
     * signed in, {@code pkg publish} says so. {@code pkg login --key} takes an
     * API key made on the account page; the sign-in goes to the computer's
     * secret store, not onto its disk. {@code pkg publish} asks the player to
     * confirm in chat; after Allow the package is in the community channel
     * and installs like any other. {@code pkg like}, {@code pkg favorite} and
     * {@code pkg favorites} work; {@code pkg yank} takes the version out of
     * the listings; {@code pkg logout} signs out. Controls: publishing without
     * a sign-in; a Deny in chat publishes nothing; the same version cannot be
     * published twice.
     */
    static Scenario account() {
        var b = Scenario.builder("repo_account",
                        "a player signs in with an API key (kept in the computer's secret store), publishes an app (confirmed in chat), installs,"
                                + " likes and favorites it, yanks it and signs out; nothing is published without a sign-in or after Deny")
                .asPlayer()
                .timeLimit(58_000);
        b.host("pc", new BlockPos(0, 1, 0), null);
        b.decor(settings("open"));
        b.note("Setup (done for you): one computer. The server's config has repo.access = open. The player has an account on the"
                + " repository's website and made an API key on its account page.");
        b.await(r -> {
            String e = ready(r, true);
            if (e == null && env("ECM_TEST_REPO_APIKEY").isEmpty()) {
                r.fail("ECM_TEST_REPO_APIKEY is not set (scripts/repo-backend-local.sh up writes it)");
                return "no API key";
            }
            return e;
        }, "the repository's backend is up, and the player has an API key", 2_000);

        b.note("pc: write an app");
        run(b, "pc", "mkdir -p /src/mine", PROMPT, "a folder for the app");
        b.sendFn("pc", r -> "echo 'id = \"" + myId(r) + "\"' > /src/mine/app.toml", "echo 'id = \"com.ecmtest.r<a number of your own>\"' > /src/mine/app.toml");
        b.expect("pc", PROMPT, "pc: shell");
        for (String line : new String[] {"name = \"Mine\"", "version = \"1.0.0\"", "entry = \"main.lua\"", "summary = \"Published from the game\""}) {
            run(b, "pc", "echo '" + line + "' >> /src/mine/app.toml", PROMPT, "app.toml");
        }
        run(b, "pc", "echo 'print(\"mine works\")' > /src/mine/main.lua", PROMPT, "main.lua");

        b.note("pc: control - not signed in");
        run(b, "pc", "pkg whoami", "^Not signed in to ", "nobody is signed in");
        run(b, "pc", "pkg publish /src/mine", "^pkg: sign in first: pkg login ", "control: publishing needs a sign-in", "^Published");

        b.note("pc: sign in with the API key; it is kept in the secret store, not on the disk");
        b.sendFn("pc", r -> "pkg login --key " + env("ECM_TEST_REPO_APIKEY"), "pkg login --key <the API key from the account page>");
        b.expect("pc", "^Signed in to \\S+ as tester\\.", "signed in as tester", "^pkg: ");
        b.expect("pc", "^The sign-in is kept in the computer's secret store\\.", "kept in the secret store");
        b.expect("pc", PROMPT, "pc: shell");
        run(b, "pc", "cat /etc/pkg/account", "No such file", "control: no sign-in file on the disk");
        run(b, "pc", "pkg whoami", "^tester at \\S+ \\(may: ", "pkg whoami asks the repository");

        b.note("pc: control - publish, then Deny in chat: nothing is published");
        b.send("pc", "pkg publish /src/mine");
        confirm(b, false);
        b.expect("pc", "^pkg: not confirmed", "control: Deny publishes nothing", "^Published");
        b.expect("pc", PROMPT, "pc: shell");

        b.note("pc: publish, Allow in chat");
        b.send("pc", "pkg publish /src/mine");
        confirm(b, true);
        b.expect("pc", "^Published com\\.ecmtest\\.r\\d+ 1\\.0\\.0 in the community channel\\.", "published under the player's account", "^pkg: ");
        b.expect("pc", PROMPT, "pc: shell");
        b.send("pc", "pkg update");
        b.expect("pc", "^community: \\d+ packages", "the community index is refreshed");
        b.expect("pc", PROMPT, "pc: shell");
        b.sendFn("pc", r -> "pkg install " + myId(r), "pkg install <the id>");
        b.expect("pc", "^Installed Mine 1\\.0\\.0 \\(com\\.ecmtest\\.r\\d+\\) - verified publisher", "it installs from the community channel like any package", "^pkg: ");
        b.expect("pc", PROMPT, "pc: shell");
        b.sendFn("pc", r -> "lua /apps/" + myId(r) + "/main.lua", "lua /apps/<the id>/main.lua");
        b.expect("pc", "^mine works$", "and runs");
        b.expect("pc", PROMPT, "pc: shell");

        b.note("pc: like and favorite it");
        b.sendFn("pc", r -> "pkg like " + myId(r), "pkg like <the id>");
        b.expect("pc", "^Liked com\\.ecmtest\\.r\\d+\\.", "liked", "^pkg: ");
        b.expect("pc", PROMPT, "pc: shell");
        b.sendFn("pc", r -> "pkg favorite " + myId(r), "pkg favorite <the id>");
        b.expect("pc", "^com\\.ecmtest\\.r\\d+ is in your favorites\\.", "in the favorites", "^pkg: ");
        b.expect("pc", PROMPT, "pc: shell");
        b.send("pc", "pkg favorites");
        b.await(r -> Pattern.compile(q(myId(r)) + " 1\\.0\\.0  Mine  \\[app, 1 likes, \\d+ downloads, installed 1\\.0\\.0\\]").matcher(r.screen("pc")).find() ? null : "not listed",
                "pkg favorites lists it with its like and that it is installed", 10_000);
        b.expect("pc", PROMPT, "pc: shell");

        b.note("pc: control - the same version again; then yank it and sign out");
        b.send("pc", "pkg publish /src/mine");
        confirm(b, true);
        b.expect("pc", "is already published; raise the version in app\\.toml", "control: a version is published once", "^Published");
        b.expect("pc", PROMPT, "pc: shell");
        b.sendFn("pc", r -> "pkg yank " + myId(r) + " 1.0.0", "pkg yank <the id> 1.0.0");
        b.expect("pc", "^Yanked com\\.ecmtest\\.r\\d+ 1\\.0\\.0: it is no longer listed\\.", "yanked", "^pkg: ");
        b.expect("pc", PROMPT, "pc: shell");
        run(b, "pc", "pkg logout", "^Signed out of \\S+ \\(tester\\)\\.", "signed out");
        run(b, "pc", "pkg whoami", "^Not signed in to ", "nobody is signed in again");
        return b.build();
    }

    // ------------------------------------------------------------------ repo_login (the browser)

    private static final Pattern CSRF = Pattern.compile("name=\"csrf\" value=\"([^\"]+)\"");

    /**
     * What the player does outside the game: open the sign-in page in their
     * browser, sign in, and approve the code. Done here as a browser does it,
     * over HTTP, with the account's email and password.
     */
    static String approveInBrowser(String code) {
        try {
            var http = java.net.http.HttpClient.newBuilder().followRedirects(java.net.http.HttpClient.Redirect.NEVER).build();
            // The browser's cookie jar (the site's session and form-token cookies).
            Map<String, String> jar = new LinkedHashMap<>();
            java.util.function.BiFunction<String, String, java.net.http.HttpResponse<String>> go = (path, form) -> {
                try {
                    var q = java.net.http.HttpRequest.newBuilder(java.net.URI.create(url() + path));
                    if (!jar.isEmpty()) {
                        StringBuilder c = new StringBuilder();
                        jar.forEach((k, v) -> c.append(c.length() == 0 ? "" : "; ").append(k).append('=').append(v));
                        q.header("Cookie", c.toString());
                    }
                    if (form == null) q.GET();
                    else q.header("Content-Type", "application/x-www-form-urlencoded").POST(java.net.http.HttpRequest.BodyPublishers.ofString(form));
                    var answer = http.send(q.build(), java.net.http.HttpResponse.BodyHandlers.ofString());
                    for (String set : answer.headers().allValues("set-cookie")) {
                        String pair = set.split(";", 2)[0];
                        int eq = pair.indexOf('=');
                        if (eq > 0) jar.put(pair.substring(0, eq).trim(), pair.substring(eq + 1).trim());
                    }
                    return answer;
                } catch (Exception e) {
                    throw new IllegalStateException(e);
                }
            };
            java.util.function.Function<String, String> enc = s -> java.net.URLEncoder.encode(s, java.nio.charset.StandardCharsets.UTF_8);
            Matcher m = CSRF.matcher(go.apply("/signin", null).body());
            if (!m.find()) return "the sign-in page has no form";
            var in = go.apply("/signin", "csrf=" + enc.apply(m.group(1)) + "&email=" + enc.apply(env("ECM_TEST_REPO_EMAIL")) + "&password="
                    + enc.apply(env("ECM_TEST_REPO_PASSWORD")));
            if (in.statusCode() != 303) return "signing in on the website answered " + in.statusCode();
            var page = go.apply("/device?code=" + enc.apply(code), null);
            m = CSRF.matcher(page.body());
            if (page.statusCode() != 200 || !page.body().contains("Connect a computer") || !m.find())
                return "the device page did not offer code " + code + " (http " + page.statusCode() + ")";
            var ok = go.apply("/device", "csrf=" + enc.apply(m.group(1)) + "&code=" + enc.apply(code) + "&action=approve");
            return ok.statusCode() == 200 ? null : "approving answered " + ok.statusCode();
        } catch (RuntimeException e) {
            return "the browser steps failed: " + e;
        }
    }

    /**
     * {@code repo_login}: {@code pkg login} with no key shows a short code and
     * the address of the repository's sign-in page, and puts a clickable link
     * in the player's chat. The player opens it in their real browser, signs
     * in with their email and password (never typed in the game) and approves
     * the code; {@code pkg} notices by itself and is signed in. Control: before
     * the code is approved {@code pkg} is still waiting, and nobody is signed in.
     */
    static Scenario login() {
        var b = Scenario.builder("repo_login",
                        "pkg login shows a code and puts the sign-in page's link in the player's chat; the player approves the code in their"
                                + " browser and pkg is signed in by itself")
                .asPlayer()
                .timeLimit(58_000);
        b.host("pc", new BlockPos(0, 1, 0), null);
        b.decor(settings("open"));
        b.note("Setup (done for you): one computer; repo.access = open. The player has an account on the repository's website.");
        b.await(r -> {
            String e = ready(r, true);
            if (e == null && env("ECM_TEST_REPO_EMAIL").isEmpty()) {
                r.fail("ECM_TEST_REPO_EMAIL and ECM_TEST_REPO_PASSWORD are not set (scripts/repo-backend-local.sh up writes them)");
                return "no account";
            }
            return e;
        }, "the repository's backend is up, and the player has an account", 2_000);
        run(b, "pc", "pkg whoami", "^Not signed in to ", "nobody is signed in");
        b.mutate(r -> r.player().clearHeard(), "(the player's chat is empty)");
        b.send("pc", "pkg login");
        b.expect("pc", "^To sign in, open this page in your browser and enter the code:", "pkg says what to do", "^pkg: ");
        b.expect("pc", "^  code: [A-Z0-9]{4}-[A-Z0-9]{4}$", "and shows the code");
        b.expect("pc", "^Waiting for you to approve it there", "pkg waits");
        b.await(r -> r.player().heard().stream().anyMatch(s -> s.contains(url() + "/device?code=")) ? null : "no link in chat: " + r.player().heard(),
                "the sign-in page's link is in the player's chat (clicking it opens their browser)", 10_000);
        b.await(r -> r.screen("pc").contains("Signed in to") ? "signed in without approval!" : null, "control: still waiting, nobody signed in yet", 1_000);
        b.note("Outside the game: the player opens the page, signs in with email and password, and approves the code");
        b.mutate(r -> {
            Matcher m = Pattern.compile("(?m)^  code: ([A-Z0-9]{4}-[A-Z0-9]{4})\\s*$").matcher(r.screen("pc"));
            String why = m.find() ? approveInBrowser(m.group(1)) : "no code on the screen";
            if (why != null) r.fail(why);
        }, "(in the browser) sign in, enter the code, Approve");
        b.expect("pc", "^Signed in to \\S+ as tester\\.", "pkg noticed and is signed in", "^pkg: ");
        b.expect("pc", PROMPT, "pc: shell");
        run(b, "pc", "pkg whoami", "^tester at ", "pkg whoami confirms it");
        run(b, "pc", "pkg logout", "^Signed out of ", "signed out again");
        return b.build();
    }

    static {
        for (Scenario s : new Scenario[] {open(), internet(), datacenter(false), datacenter(true), account(), login()}) ALL.put(s.name, s);
    }

    private RepoBackendScenarios() {}
}
//?}
