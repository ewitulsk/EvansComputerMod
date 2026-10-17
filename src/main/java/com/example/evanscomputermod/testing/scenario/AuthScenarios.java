package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Accounts (docs/AUTH.md): an Auth server ({@code authd}) and the backends that trust it, with the
 * players' apps and the console {@code chat}, spawnable with {@code /ecm scenario spawn <name>}; also
 * the {@code ecm_discord} GameTests of the same names.
 *
 * <p>{@code auth_pinned_key}: the server runs {@code authd --name Lab} and a Discord backend that
 * trusts it ({@code discordd --auth 10.0.8.1:7600 --auth-key <fp>}, the fingerprint typed from what
 * {@code authd key} printed). alex signs up in Discord (first contact: the key is pinned); the
 * server's console signs up with {@code chat --user base --new} (its own pin). Then the operator
 * reinstalls the Auth server the way a player would: {@code kill} it, {@code rm} its key and users,
 * start it again (a new key), and restarts {@code discordd} with the new fingerprint. Discord on alex
 * shows "The identity of Lab has changed", with no sign-in button, and the backend sees no sign-in
 * from alex. {@code chat --user base} refuses the same way and names {@code chat --forget <old fp>}.
 * Controls: Settings &gt; Forget realm in Discord makes it a first contact again and sign-up works;
 * {@code chat --forget <old fp>} (the pin removed by hand) lets {@code chat} sign up again.
 *
 * <p>{@code auth_shared_realm}: one {@code authd} ("Village Accounts") trusted by two Discord
 * backends (Lab and Village) and by {@code musicd} ("Village Radio"); a third Discord backend,
 * Other, keeps its own accounts. alex signs up once, on Lab: Village signs in by itself (a refresh
 * token, no password). Control: Other, another realm, still asks ("First time here"). Then alex
 * opens Music: it keeps its own logins, so it asks too (first contact for Music), and alex signs in
 * there with the same account.
 */
public final class AuthScenarios {

    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static final String DISCORD = AppScenarios.DISCORD, MUSIC = "dev.ecm.music";
    static final String SERVER_IP = "10.0.8.1";
    static final String FP = "([0-9a-f]{32})";

    static {
        for (Scenario s : List.of(pinnedKey(), sharedRealm())) ALL.put(s.name, s);
    }

    private AuthScenarios() {}

    // ------------------------------------------------------------------ helpers

    /** The fingerprint {@code authd key} printed last on the server. */
    static String realmFp(ScenarioRun r) {
        return CellNetScenarios.last(r.screen("server"), "^fingerprint " + FP + "\\s*$");
    }

    /** The pid of {@code prog} in the server's last {@code ps}. */
    static String pid(ScenarioRun r, String prog) {
        return CellNetScenarios.last(r.screen("server"), "^\\s*(\\d+)\\s+\\S+\\s+" + prog + "(?:\\.wasm)?\\b.*$");
    }

    /** {@code authd key}: read its fingerprint off the screen (the operator gives it to the backends). */
    static void authdKey(Scenario.Builder b) {
        b.send("server", "authd key");
        b.expect("server", "^fingerprint [0-9a-f]{32}", "authd key prints the realm's fingerprint", "^authd: ");
        b.expect("server", PlayerKit.PROMPT, "shell");
    }

    /** Stop {@code prog} on the server: ps, kill its pid. */
    static void stop(Scenario.Builder b, String prog) {
        // (until: a line typed while the shell is still taking back the console after chat is sent again)
        b.until("server", "ps", "^\\s*\\d+\\s+\\S+\\s+" + prog, "ps lists " + prog);
        b.sendFn("server", r -> "kill " + pid(r, prog), "kill <" + prog + "'s pid>");
        b.expect("server", "^Process killed\\.", prog + " stopped");
    }

    /** A Discord backend that trusts the Auth server (the fingerprint read from the screen). */
    static void sharedDiscordd(Scenario.Builder b, String args, String name) {
        b.sendFn("server", r -> "discordd " + args + " --auth " + SERVER_IP + ":7600 --auth-key " + realmFp(r) + " &",
                "discordd " + args + " --auth " + SERVER_IP + ":7600 --auth-key <the fingerprint authd key printed> &");
        b.expect("server", "discordd listening on port \\d+ .*\"" + name + "\"", "backend " + name + " listening", "^discordd: ");
    }

    /** Null once {@code host}'s Discord window has no text exactly {@code text} (no such button). */
    static String noButton(ScenarioRun r, String host, String text) {
        var c = DiscordScenarios.comp(r, host);
        if (c == null) return "no GUI on " + host;
        return c.findText("Discord", text) == null ? null : host + "'s Discord shows a \"" + text + "\" button";
    }

    /** The server's screen since the last "discordd listening" line. */
    static String sinceRestart(ScenarioRun r) {
        String s = r.screen("server");
        int at = s.lastIndexOf("discordd listening");
        return at < 0 ? s : s.substring(at);
    }

    static Scenario.Builder twoComputers(String name, String what) {
        var b = Scenario.builder(name, what).asPlayer();
        b.host("server", DiscordScenarios.SERVER, "-");
        b.host("alex", DiscordScenarios.ALEX, "-");
        b.link("lan_a", "server", Direction.DOWN, "alex", Direction.DOWN, Scenario.Path.from(new BlockPos(0, 0, 0)).go(Direction.EAST, 4));
        b.note("Setup (done for you): a server cabled to alex below (eth0).");
        b.send("server", "ifconfig eth0 " + SERVER_IP + "/24");
        b.expect("server", "eth0: inet 10\\.0\\.8\\.1/24", "server has " + SERVER_IP);
        b.send("alex", "ifconfig eth0 10.0.8.2/24");
        b.expect("alex", "eth0: inet 10\\.0\\.8\\.2/24", "alex has 10.0.8.2");
        return b;
    }

    // ------------------------------------------------------------------ auth_pinned_key

    private static Scenario pinnedKey() {
        var b = twoComputers("auth_pinned_key",
                "an Auth server and a Discord backend that trusts it; alex signs up in Discord and the console signs up with chat;"
                        + " the Auth server is reinstalled with a new key: Discord and chat refuse it until the old pin is removed by hand");
        b.note("server: the Auth server (authd), its fingerprint, a Discord backend that trusts that key, disco serve, the Discord package");
        b.send("server", "authd --name Lab &");
        b.expect("server", "authd listening on port 7600, realm \"Lab\"", "authd listening (a new realm key made in /var/auth)", "^authd: ");
        authdKey(b);
        sharedDiscordd(b, "", "Discord");
        b.expect("server", "accounts: \"Lab\" at 10\\.0\\.8\\.1:7600", "discordd trusts the realm Lab (it checked the key's fingerprint)");
        b.send("server", "disco serve &");
        b.expect("server", "disco: answering on UDP port 5354", "disco serve answers");
        DiscordScenarios.repository(b);

        b.note("alex: Discord finds the backend; first contact with Lab: sign up (the key is pinned)");
        DiscordScenarios.openDiscord(b, "alex", SERVER_IP);
        b.await(r -> DiscordScenarios.shows(r, "alex", "First time here"), "first contact: Lab's name, address and key are shown", 25_000);
        DiscordScenarios.signUp(b, "alex", "alex");
        b.expect("server", "signup alex from 10\\.0\\.8\\.2", "authd makes alex's account");
        b.await(r -> DiscordScenarios.shows(r, "alex", "# general"), "alex is signed in: the home server opens", 20_000);

        b.note("server: the console signs up too, with chat (its own pin of Lab)");
        DiscordScenarios.chatSignUp(b, "server", "chat --user base --new " + SERVER_IP, "base");
        b.expect("server", "Joined Discord as base", "chat joins #general as base");
        b.send("server", "/quit");
        b.expect("server", "\\*\\*\\* bye", "chat quits");

        b.note("The operator reinstalls the Auth server: stop it, delete its key and accounts, start it again (a new key);"
                + " discordd is restarted with the new fingerprint");
        stop(b, "authd");
        b.send("server", "rm /var/auth/realm.key /var/auth/users");
        b.expect("server", PlayerKit.PROMPT, "the key and the accounts are gone", "^rm: ");
        b.send("server", "authd --name Lab &");
        b.expect("server", "made a new realm key in /var/auth/realm\\.key", "authd makes a new key", "^authd: ");
        authdKey(b);
        stop(b, "discordd");
        sharedDiscordd(b, "", "Discord");
        b.expect("server", "trusting the realm \"Lab\"", "discordd fetched and checked the new key");

        b.note("alex: Discord reconnects; Lab presents another key: refused, with the warning and no way to sign in");
        b.await(r -> DiscordScenarios.shows(r, "alex", "The identity of Lab has changed"),
                "Discord warns: \"The identity of Lab has changed. This could be someone pretending to be it.\"", 40_000);
        b.await(r -> DiscordScenarios.shows(r, "alex", "Remembered key:"), "both fingerprints are shown (remembered and now)", 5_000);
        b.await(r -> noButton(r, "alex", "Sign in"), "no Sign in button on the warning", 2_000);
        b.await(r -> noButton(r, "alex", "Create account"), "no Create account button either", 2_000);
        b.await(r -> sinceRestart(r).contains("signed in from") ? "the backend saw a sign-in" : null,
                "control: the backend sees no sign-in from alex (no HELLO with the old login)", 3_000);

        b.note("server: chat --user base refuses the changed key the same way and names chat --forget");
        b.send("server", "chat --user base " + SERVER_IP);
        b.expect("server", "the identity of Lab has changed",
                "chat refuses: the identity of Lab has changed", "Trust it and sign in|Password for");
        b.expect("server", "remove the old pin with: chat --forget [0-9a-f]{32}", "chat says how to remove the old pin by hand");
        b.expect("server", "Process exited with code 1", "chat exits without signing in");

        b.note("Control: the pin removed by hand. chat --forget <old fp>, then chat signs up again");
        b.sendFn("server", r -> "chat --forget " + CellNetScenarios.last(r.screen("server"), "chat --forget " + FP),
                "chat --forget <the old fingerprint chat printed>");
        b.expect("server", "chat: forgot the realm", "chat forgets the old pin and its login");
        DiscordScenarios.chatSignUp(b, "server", "chat --user base --new " + SERVER_IP, "base");
        b.expect("server", "Joined Discord as base", "chat joins with the new key (first contact again)");
        b.send("server", "/quit");
        b.expect("server", "\\*\\*\\* bye", "chat quits");

        b.note("Control: Discord's Settings > Forget realm (by hand); then it is a first contact and sign-up works");
        b.await(r -> DiscordScenarios.click(r, "alex", "Discord", "Settings"), "alex: click Settings", 10_000);
        b.await(r -> DiscordScenarios.click(r, "alex", "Discord", "Forget realm"), "alex: click Forget realm", 10_000);
        b.await(r -> DiscordScenarios.shows(r, "alex", "Not signed in."), "Settings: Lab is no longer pinned; not signed in", 25_000);
        b.await(r -> DiscordScenarios.click(r, "alex", "Discord", "Sign in"), "alex: click Sign in (in Settings)", 5_000);
        b.await(r -> DiscordScenarios.shows(r, "alex", "First time here"), "first contact with Lab's new key", 25_000);
        DiscordScenarios.signUp(b, "alex", "alex");
        b.expect("server", "alex signed in from 10\\.0\\.8\\.2", "the backend sees alex sign in with the new realm's token");
        b.await(r -> DiscordScenarios.shows(r, "alex", "# general"), "alex is in the home server again", 20_000);
        return b.timeLimit(240_000).build();
    }

    // ------------------------------------------------------------------ auth_shared_realm

    private static Scenario sharedRealm() {
        var b = twoComputers("auth_shared_realm",
                "one Auth server for two Discord backends and a music server; alex signs up once and the second backend signs in"
                        + " by itself; a backend with its own accounts asks again; Music keeps its own logins and asks too");
        b.note("server: authd \"Village Accounts\"; Discord backends Lab and Village trust it, Other keeps its own accounts;"
                + " musicd trusts it too; disco serve; the Discord and Music packages");
        b.send("server", "authd --name \"Village Accounts\" &");
        b.expect("server", "authd listening on port 7600, realm \"Village Accounts\"", "authd listening", "^authd: ");
        authdKey(b);
        sharedDiscordd(b, "7777 --name Lab --data /var/lab", "Lab");
        sharedDiscordd(b, "7790 --name Village --data /var/village", "Village");
        b.send("server", "discordd 7810 --name Other --data /var/other &");
        b.expect("server", "discordd listening on port 7810 .*\"Other\"", "backend Other listening (its own accounts)");
        b.sendFn("server", r -> "musicd --name \"Village Radio\" --auth " + SERVER_IP + ":7600 --auth-key " + realmFp(r) + " &",
                "musicd --name \"Village Radio\" --auth " + SERVER_IP + ":7600 --auth-key <the fingerprint authd key printed> &");
        b.expect("server", "musicd listening on port 7800, \"Village Radio\"", "musicd listening (accounts of Village Accounts)", "^musicd: ");
        b.send("server", "disco serve &");
        b.expect("server", "disco: answering on UDP port 5354", "disco serve answers");
        b.send("server", "discord --source /src/discord");
        b.expect("server", "discord: wrote Discord \\(dev\\.ecm\\.discord\\) 1\\.0\\.0 to /src/discord", "Discord's package folder", "^discord: can't");
        b.expect("server", PlayerKit.PROMPT, "shell");
        b.send("server", "music --source /src/music");
        b.expect("server", "music: wrote Music \\(dev\\.ecm\\.music\\) 1\\.0\\.0 to /src/music", "Music's package folder", "^music: can't");
        b.expect("server", PlayerKit.PROMPT, "shell");
        b.send("server", "pkg build /src/discord /srv/repo");
        b.expect("server", "^Built /srv/repo/dev\\.ecm\\.discord-1\\.0\\.0\\.ecmpkg", "pkg build packs Discord", "^pkg: ");
        b.expect("server", PlayerKit.PROMPT, "shell");
        b.send("server", "pkg build /src/music /srv/repo");
        b.expect("server", "^Built /srv/repo/dev\\.ecm\\.music-1\\.0\\.0\\.ecmpkg", "pkg build packs Music", "^pkg: ");
        b.expect("server", PlayerKit.PROMPT, "shell");
        b.send("server", "pkg serve /srv/repo 8080 &");
        b.expect("server", "Serving 2 packages from /srv/repo on port 8080", "the repository is online", "^pkg: ");

        b.note("alex: installs Discord and Music, opens Discord; it finds the three backends");
        b.send("alex", "pkg repo add lab http://" + SERVER_IP + ":8080/");
        b.expect("alex", "lab", "the repository is added", "^pkg: ");
        b.expect("alex", PlayerKit.PROMPT, "shell");
        for (String app : List.of(DISCORD, MUSIC)) {
            b.send("alex", "pkg install " + app);
            b.expect("alex", "^Installed \\w+ 1\\.0\\.0 \\(" + app.replace(".", "\\.") + "\\)", "alex installs " + app, "^pkg: ");
            b.expect("alex", PlayerKit.PROMPT, "shell");
        }
        b.send("alex", "gui");
        b.expect("alex", "^gui: started", "the desktop starts", "(?i)unknown command|no GUI device");
        b.await(r -> DiscordScenarios.comp(r, "alex") != null && DiscordScenarios.comp(r, "alex").shellConnected() ? null : "waiting for the shell",
                "alex's desktop is up", 15_000);
        b.await(r -> DiscordScenarios.clickOn(r, "alex", "taskbar", "Apps"), "alex: click Apps on the taskbar", 10_000);
        b.await(r -> DiscordScenarios.clickOn(r, "alex", "popup", "Discord"), "alex: click Discord in the launcher", 5_000);
        b.await(r -> DiscordScenarios.comp(r, "alex").layerTexts("permission").contains("Allow Discord to use network?") ? null
                : "no dialog: " + DiscordScenarios.comp(r, "alex").layerTexts("permission"), "Discord asks for the network", 20_000);
        b.await(r -> DiscordScenarios.clickOn(r, "alex", "permission", "Allow"), "alex: click Allow", 5_000);

        b.note("alex: Sign in to Lab, Create one; Village (the same realm) signs in by itself");
        b.await(r -> DiscordScenarios.click(r, "alex", "Discord", "Sign in to Lab"), "alex: click \"Sign in to Lab\"", 30_000);
        DiscordScenarios.signUp(b, "alex", "alex");
        b.expect("server", "signup alex from 10\\.0\\.8\\.2", "authd makes alex's account (once)");
        b.await(r -> DiscordScenarios.shows(r, "alex", "LA") == null && DiscordScenarios.shows(r, "alex", "VI") == null
                        && DiscordScenarios.lacks(r, "alex", "Sign in to Lab") == null && DiscordScenarios.lacks(r, "alex", "Sign in to Village") == null
                        ? null : DiscordScenarios.shows(r, "alex", "VI"),
                "Lab and Village are on alex's rail, both signed in (Village silently)", 25_000);
        b.expect("server", "refresh alex from 10\\.0\\.8\\.2", "authd: Village's sign-in used the refresh token, no password");

        b.note("Control: Other keeps its own accounts: it asks again");
        b.await(r -> DiscordScenarios.click(r, "alex", "Discord", "Sign in to Other"), "alex: click \"Sign in to Other\"", 10_000);
        b.await(r -> DiscordScenarios.shows(r, "alex", "First time here"), "Other is another realm: a first contact, its own sign-in", 10_000);

        b.note("Music keeps its own logins: it asks for Village Accounts too; alex signs in with the same account");
        b.await(r -> DiscordScenarios.clickOn(r, "alex", "taskbar", "Apps"), "alex: click Apps on the taskbar", 10_000);
        b.await(r -> DiscordScenarios.clickOn(r, "alex", "popup", "Music"), "alex: click Music in the launcher", 5_000);
        b.await(r -> DiscordScenarios.comp(r, "alex").layerTexts("permission").contains("Allow Music to use network?") ? null
                : "no dialog: " + DiscordScenarios.comp(r, "alex").layerTexts("permission"), "Music asks for the network", 20_000);
        b.await(r -> DiscordScenarios.clickOn(r, "alex", "permission", "Allow"), "alex: click Allow", 5_000);
        b.await(r -> DiscordScenarios.click(r, "alex", "Music", "Sign in"), "alex: click Sign in under Village Radio", 25_000);
        b.await(r -> musicShows(r, "First time here"), "Music: first contact with Village Accounts (Discord's pin isn't Music's)", 20_000);
        b.await(r -> DiscordScenarios.click(r, "alex", "Music", "Username"), "alex: click Username", 5_000);
        DiscordScenarios.typeOn(b, "alex", "alex");
        b.mutate(r -> r.pressKey("alex", DiscordScenarios.HID_TAB, 0, "\t"), "alex: Tab to Password");
        DiscordScenarios.typeOn(b, "alex", DiscordScenarios.PASSWORD);
        b.mutate(r -> r.pressKey("alex", DiscordScenarios.HID_ENTER, 0, "\n"), "alex: Enter (Sign in)");
        b.expect("server", "login alex from 10\\.0\\.8\\.2", "authd signs alex in (the same account, Music's own login)");
        b.await(r -> musicShows(r, "Signed in as"), "Music is signed in", 20_000);
        return b.timeLimit(240_000).build();
    }

    static String musicShows(ScenarioRun r, String text) {
        var c = DiscordScenarios.comp(r, "alex");
        if (c == null) return "no GUI";
        for (var w : c.windows()) {
            if (!w.appId().equals(MUSIC) || !w.mapped()) continue;
            var texts = c.textsOf(w.id());
            return texts.stream().anyMatch(t -> t.contains(text)) ? null : "Music shows " + texts;
        }
        return "no Music window";
    }
}
//?}
