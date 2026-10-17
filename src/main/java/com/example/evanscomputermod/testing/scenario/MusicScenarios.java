package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.ui.Compositor;
import com.example.evanscomputermod.speaker.SpeakerBlock;
import com.example.evanscomputermod.speaker.SpeakerBlockEntity;
import com.example.evanscomputermod.speaker.SpeakerContent;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.WeakHashMap;

/**
 * Music (docs/MUSIC.md): {@code musicd} on a server, the Music app on the players' computers,
 * spawnable with {@code /ecm scenario spawn <name>}; also the {@code ecm_apps} GameTests
 * {@code music_stream} and {@code music_playlist}, and the {@code ecm_phone} GameTest
 * {@code music_background}.
 *
 * <p>{@code music_stream}: the server has the three Tech Village songs in /srv/music and runs
 * {@code musicd} (its own accounts) and {@code disco serve}, and publishes the Music package.
 * alex installs Music with pkg and opens it: it finds "Village Radio" by itself. Control: a song
 * tapped before signing in doesn't play ("Sign in to Village Radio first"). alex makes an account
 * on the shared sign-in screen (the server's key is shown: first contact), searches "redst" (as
 * typed, "Redstone Sunrise" is found), plays it and alex's Speaker plays. Control: the server
 * stops musicd mid-song: the song plays on from what is buffered; alex plays it from the start
 * ("|<"): "Buffering..."; the server starts musicd again and it plays again.
 *
 * <p>{@code music_playlist}: alex makes a playlist "Road Trip", adds two songs from the search,
 * moves the first one down and keeps it private. Control: bo (another computer, another account) finds
 * no Road Trip, neither in the Library nor in the search. bo signs out and signs in as alex: Road
 * Trip is there in the new order and plays on bo's Speaker.
 *
 * <p>Exception (docs/SCENARIO_EXCEPTIONS.md): the MP3s are copied into the server's /srv/music
 * (binary files can't be typed; an owner brings songs in from outside the game).
 */
public final class MusicScenarios {
    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static final String MUSIC = "dev.ecm.music";
    static final String SERVER_IP = "10.0.8.1";
    static final List<String> SONGS = List.of("01-redstone-sunrise.mp3", "02-village-fiber.mp3", "03-data-center-nights.mp3");
    static final String SONG_DIR = "/evanscomputermod/village/music/";
    static final BlockPos ALEX_SPEAKER = new BlockPos(5, 1, 0), BO_SPEAKER = new BlockPos(-1, 1, 30);
    static final int HID_TAB = 0x2B, HID_ENTER = 0x28;

    static final class State {
        long samples;
        long at;
    }

    private static final Map<ScenarioRun, State> STATES = java.util.Collections.synchronizedMap(new WeakHashMap<>());

    static State st(ScenarioRun r) {
        return STATES.computeIfAbsent(r, k -> new State());
    }

    private MusicScenarios() {}

    // ------------------------------------------------------------------ the Music window on a computer

    static Compositor comp(ScenarioRun r, String host) {
        return DiscordScenarios.comp(r, host);
    }

    /** Click {@code text} in {@code host}'s Music window. */
    static String click(ScenarioRun r, String host, String text) {
        return DiscordScenarios.click(r, host, "Music", text);
    }

    /** Null once {@code host}'s Music window shows a text containing {@code text}. */
    static String shows(ScenarioRun r, String host, String text) {
        var c = comp(r, host);
        if (c == null) return "no GUI on " + host;
        for (var w : c.windows()) {
            if (!w.appId().equals(MUSIC) || !w.mapped()) continue;
            var texts = c.textsOf(w.id());
            return texts.stream().anyMatch(t -> t.contains(text)) ? null : host + "'s Music shows " + texts;
        }
        return "no Music window on " + host;
    }

    static String lacks(ScenarioRun r, String host, String text) {
        return shows(r, host, text) == null ? host + "'s Music shows " + text : null;
    }

    static void typeOn(Scenario.Builder b, String host, String text) {
        b.mutate(r -> r.typeText(host, text), host + ": type '" + text + "'");
    }

    static long drained(ScenarioRun r, BlockPos speaker) {
        return r.level().getBlockEntity(r.abs(speaker)) instanceof SpeakerBlockEntity s ? s.audio().samplesDrained() : -1;
    }

    /** The Speaker at {@code speaker} plays: at least a quarter of 22050 samples a second over {@code ms}. */
    static void plays(Scenario.Builder b, BlockPos speaker, int ms, String what) {
        b.mutate(r -> {
            st(r).samples = drained(r, speaker);
            st(r).at = System.currentTimeMillis();
        }, "count the Speaker's samples");
        b.waitMs(ms, "listen for " + ms / 1000.0 + " s");
        b.await(r -> {
            long got = drained(r, speaker) - st(r).samples;
            long want = (System.currentTimeMillis() - st(r).at) * 22050 / 4000;
            r.say("§f  " + got + " samples in " + (System.currentTimeMillis() - st(r).at) + " ms");
            return got >= want ? null : "only " + got + " samples (want " + want + ")";
        }, what, 3_000);
    }

    /** Click Allow on every permission Music asks for until {@code done} holds. */
    static String allowAll(ScenarioRun r, String host, java.util.function.Function<ScenarioRun, String> done) {
        var c = comp(r, host);
        if (c == null) return "no GUI";
        var texts = c.layerTexts("permission");
        if (texts.stream().anyMatch(t -> t.contains("Allow Music"))) {
            String why = DiscordScenarios.clickOn(r, host, "permission", "Allow");
            return why == null ? "allowed " + texts : why;
        }
        return done.apply(r);
    }

    // ------------------------------------------------------------------ steps

    /** The server: the songs, musicd, disco serve and the Music package in a repository. */
    static void server(Scenario.Builder b) {
        b.mutate(r -> PlayerKit.dropFiles(r, "server", "srv/music", SONG_DIR, SONGS),
                "copy the three Tech Village songs into the server's /srv/music [exception: binary files copied in; docs/SCENARIO_EXCEPTIONS.md]");
        b.send("server", "musicd --name \"Village Radio\" &");
        b.expect("server", "musicd listening on port 7800 \\(sign-in on 7801\\), \"Village Radio\"", "musicd listens (its own accounts on 7801)", "^musicd: ");
        b.expect("server", "scan: 3 songs \\(3 new", "musicd scans the library: three songs");
        b.send("server", "disco serve &");
        b.expect("server", "disco: answering on UDP port 5354", "disco serve answers for musicd");
        b.send("server", "music --source /src/music");
        b.expect("server", "music: wrote Music \\(dev\\.ecm\\.music\\) 1\\.0\\.0 to /src/music", "Music's package folder", "^music: can't");
        b.expect("server", PlayerKit.PROMPT, "shell");
        b.send("server", "pkg build /src/music /srv/repo");
        b.expect("server", "^Built /srv/repo/dev\\.ecm\\.music-1\\.0\\.0\\.ecmpkg", "pkg build packs it", "^pkg: ");
        b.expect("server", PlayerKit.PROMPT, "shell");
        b.send("server", "pkg serve /srv/repo 8080 &");
        b.expect("server", "Serving 1 packages from /srv/repo on port 8080", "the repository is online", "^pkg: ");
    }

    /** {@code host}: install Music with pkg, start the GUI, open Music from the launcher, allow it the network. */
    static void openMusic(Scenario.Builder b, String host, String repoIp) {
        b.send(host, "pkg repo add lab http://" + repoIp + ":8080/");
        b.expect(host, "lab", "the repository is added", "^pkg: ");
        b.expect(host, PlayerKit.PROMPT, "shell");
        b.send(host, "pkg install " + MUSIC);
        b.expect(host, "^Installed Music 1\\.0\\.0 \\(dev\\.ecm\\.music\\)", host + " installs Music over the network", "^pkg: ");
        b.expect(host, PlayerKit.PROMPT, "shell");
        b.send(host, "gui");
        b.expect(host, "^gui: started", "the desktop starts", "(?i)unknown command|no GUI device");
        b.await(r -> comp(r, host) != null && comp(r, host).shellConnected() ? null : "waiting for the shell", host + "'s desktop is up", 15_000);
        b.await(r -> DiscordScenarios.clickOn(r, host, "taskbar", "Apps"), host + ": click Apps on the taskbar", 10_000);
        b.await(r -> DiscordScenarios.clickOn(r, host, "popup", "Music"), host + ": click Music in the launcher", 5_000);
        b.await(r -> comp(r, host).layerTexts("permission").contains("Allow Music to use network?") ? null
                : "no dialog: " + comp(r, host).layerTexts("permission"), host + ": Music asks for the network as it looks for servers", 20_000);
        b.await(r -> DiscordScenarios.clickOn(r, host, "permission", "Allow"), host + ": click Allow", 5_000);
        b.await(r -> shows(r, host, "Village Radio"), host + ": Music finds Village Radio by itself", 25_000);
    }

    /** On the open sign-in screen: Create account, the username, Tab Tab, the password twice, Enter. */
    static void signUp(Scenario.Builder b, String host, String user) {
        b.await(r -> shows(r, host, "First time here"), host + ": first contact: the server's name, address and key", 20_000);
        b.await(r -> click(r, host, "No account yet? Create one"), host + ": click \"No account yet? Create one\"", 5_000);
        b.await(r -> shows(r, host, "Create an account"), host + ": the form asks for a new account", 5_000);
        b.await(r -> click(r, host, "Username"), host + ": click Username", 5_000);
        typeOn(b, host, user);
        b.mutate(r -> {
            r.pressKey(host, HID_TAB, 0, "\t");
            r.pressKey(host, HID_TAB, 0, "\t");
        }, host + ": Tab, Tab (past the optional display name) to Password");
        typeOn(b, host, DiscordScenarios.PASSWORD);
        b.mutate(r -> r.pressKey(host, HID_TAB, 0, "\t"), host + ": Tab to Password again");
        typeOn(b, host, DiscordScenarios.PASSWORD);
        b.mutate(r -> r.pressKey(host, HID_ENTER, 0, "\n"), host + ": Enter (Create account)");
        b.expect("server", "signup " + user + " from ", "musicd's realm makes the account");
        b.await(r -> shows(r, host, "Signed in as"), host + " is signed in", 20_000);
        b.await(r -> click(r, host, "Done"), host + ": click Done", 5_000);
    }

    /** Sign in with an existing account: Username, Tab, Password, Enter. */
    static void signIn(Scenario.Builder b, String host, String user) {
        b.await(r -> click(r, host, "Username"), host + ": click Username", 20_000);
        typeOn(b, host, user);
        b.mutate(r -> r.pressKey(host, HID_TAB, 0, "\t"), host + ": Tab to Password");
        typeOn(b, host, DiscordScenarios.PASSWORD);
        b.mutate(r -> r.pressKey(host, HID_ENTER, 0, "\n"), host + ": Enter (Sign in)");
        b.expect("server", "login " + user + " from ", "musicd's realm signs " + user + " in");
        b.await(r -> shows(r, host, "Signed in as"), host + " is signed in", 20_000);
        b.await(r -> click(r, host, "Done"), host + ": click Done", 5_000);
    }

    /** Search tab, click the field, type {@code q}; wait for {@code finds}. */
    static void search(Scenario.Builder b, String host, String q, String finds) {
        b.await(r -> click(r, host, "Search"), host + ": click Search", 5_000);
        b.await(r -> click(r, host, "Songs, artists, albums, playlists"), host + ": click the search field", 5_000);
        typeOn(b, host, q);
        // A song is clicked only once the results are in (the top result shows), not in the page before.
        String top = finds.startsWith("Nothing") ? finds : "TOP RESULT";
        b.await(r -> shows(r, host, top) == null ? shows(r, host, finds) : shows(r, host, top), host + ": typing '" + q + "' finds " + finds, 10_000);
    }

    static Scenario.Builder twoComputers(String name, String what) {
        var b = Scenario.builder(name, what).asPlayer();
        b.host("server", DiscordScenarios.SERVER, "-");
        b.host("alex", DiscordScenarios.ALEX, "-");
        b.link("lan_a", "server", Direction.DOWN, "alex", Direction.DOWN, Scenario.Path.from(new BlockPos(0, 0, 0)).go(Direction.EAST, 4));
        b.note("Setup (done for you): a server cabled to alex below (eth0).");
        b.send("server", "ifconfig eth0 10.0.8.1/24");
        b.expect("server", "eth0: inet 10\\.0\\.8\\.1/24", "server has 10.0.8.1 on alex's LAN");
        b.send("alex", "ifconfig eth0 10.0.8.2/24");
        b.expect("alex", "eth0: inet 10\\.0\\.8\\.2/24", "alex has 10.0.8.2");
        return b;
    }

    static Scenario.Decor speakers(List<BlockPos> at) {
        return PlayerKit.decor(at, null, r -> {
            for (BlockPos p : at) r.player().place(SpeakerContent.SPEAKER.get(), r.abs(p), Direction.NORTH, s -> s.getValue(SpeakerBlock.FACING) == Direction.NORTH);
        }, null);
    }

    /** The pid of {@code prog} in the server's last {@code ps}. */
    static String pid(ScenarioRun r, String prog) {
        return CellNetScenarios.last(r.screen("server"), "^\\s*(\\d+)\\s+\\S+\\s+" + prog + "(?:\\.wasm)?\\b.*$");
    }

    // ------------------------------------------------------------------ music_stream

    private static Scenario stream() {
        var b = twoComputers("music_stream", "musicd on a server; the Music app finds it, signs up, finds 'Redstone Sunrise' as typed"
                + " and plays it on the Speaker; musicd stopped mid-song: Buffering..., then it plays again");
        b.decor(speakers(List.of(ALEX_SPEAKER)));
        b.note("Also placed for you: a Speaker east of alex.");
        b.realTime();
        server(b);
        openMusic(b, "alex", SERVER_IP);
        b.await(r -> shows(r, "alex", "Redstone Sunrise"), "alex: Home lists what is new on the server (browsing works signed out)", 15_000);
        b.note("Control: playing needs an account");
        b.await(r -> click(r, "alex", "Redstone Sunrise"), "alex: click Redstone Sunrise", 5_000);
        b.await(r -> shows(r, "alex", "Sign in to Village Radio first"), "control: it doesn't play: 'Sign in to Village Radio first'", 10_000);
        b.await(r -> drained(r, ALEX_SPEAKER) <= 0 ? null : "the Speaker played", "control: the Speaker is silent", 2_000);
        b.note("alex: Sign in, Create account on the shared sign-in screen");
        b.await(r -> click(r, "alex", "Sign in"), "alex: click Sign in (under Village Radio)", 5_000);
        signUp(b, "alex", "alex");
        b.await(r -> lacks(r, "alex", "Sign in"), "alex's rail shows Village Radio signed in", 10_000);
        search(b, "alex", "redst", "Redstone Sunrise");
        b.await(r -> click(r, "alex", "Redstone Sunrise"), "alex: click Redstone Sunrise", 5_000);
        b.await(r -> allowAll(r, "alex", x -> drained(x, ALEX_SPEAKER) > 22_050 ? null : "speaker " + drained(x, ALEX_SPEAKER)),
                "alex: allow Music the speaker; Redstone Sunrise plays on alex's Speaker", 25_000);
        plays(b, ALEX_SPEAKER, 2_000, "it keeps playing");
        b.note("Control: the server stops musicd mid-song");
        b.send("server", "ps");
        b.expect("server", "^\\s*\\d+\\s+\\S+\\s+musicd", "ps lists musicd");
        b.sendFn("server", r -> "kill " + pid(r, "musicd"), "kill <musicd's pid>");
        b.expect("server", "^Process killed\\.", "musicd stopped");
        plays(b, ALEX_SPEAKER, 2_000, "the song plays on from what Music has buffered (30 s ahead)");
        b.await(r -> click(r, "alex", "Playing"), "alex: click Playing (Now Playing)", 5_000);
        b.await(r -> click(r, "alex", "|<"),"alex: click |< (play it from the start: Music has to fetch it again)", 5_000);
        b.await(r -> shows(r, "alex", "Buffering"), "alex: 'Buffering...' while the server is down", 10_000);
        b.mutate(r -> st(r).samples = drained(r, ALEX_SPEAKER), "note the Speaker");
        b.waitMs(1_500, "a moment");
        b.await(r -> drained(r, ALEX_SPEAKER) - st(r).samples < 2_000 ? null : "the Speaker played " + (drained(r, ALEX_SPEAKER) - st(r).samples),
                "nothing plays while it buffers", 2_000);
        b.send("server", "musicd --name \"Village Radio\" &");
        b.expect("server", "musicd listening on port 7800", "musicd is back");
        b.await(r -> lacks(r, "alex", "Buffering"), "alex: Music reconnects by itself (no more Buffering...)", 25_000);
        plays(b, ALEX_SPEAKER, 2_000, "Redstone Sunrise plays again");
        return b.timeLimit(150_000).build();
    }

    // ------------------------------------------------------------------ music_playlist

    private static Scenario playlist() {
        var b = DiscordScenarios.threeComputers("music_playlist", "alex makes a private playlist with two songs and reorders it;"
                + " bo's account can't see it; signed in as alex, bo plays it");
        b.decor(speakers(List.of(BO_SPEAKER)));
        b.note("Also placed for you: a Speaker beside bo.");
        b.realTime();
        server(b);
        openMusic(b, "alex", SERVER_IP);
        b.await(r -> click(r, "alex", "Sign in"), "alex: click Sign in", 10_000);
        signUp(b, "alex", "alex");
        b.note("alex: a new playlist from the Library");
        b.await(r -> click(r, "alex", "Library"), "alex: click Library", 5_000);
        b.await(r -> click(r, "alex", "New playlist"), "alex: click New playlist", 5_000);
        b.await(r -> click(r, "alex", "Playlist name"), "alex: click the name field", 5_000);
        typeOn(b, "alex", "Road Trip");
        b.await(r -> click(r, "alex", "Create"), "alex: click Create", 5_000);
        b.await(r -> shows(r, "alex", "Made Road Trip"), "alex: 'Made Road Trip'", 10_000);
        b.note("alex: two songs from the search, each with + and Road Trip");
        String[][] picks = {{"fiber", "Village Fiber", ""}, {"nights", "Data Center Nights", "fiber"}};
        for (String[] s : picks) {
            b.await(r -> click(r, "alex", "Search"), "alex: click Search", 5_000);
            b.await(r -> s[2].isEmpty() ? click(r, "alex", "Songs, artists, albums, playlists") : click(r, "alex", s[2]),
                    "alex: click the search field", 5_000);
            b.mutate(r -> {
                r.pressKey("alex", 0x4D, 0, "");
                for (int i = 0; i < 8; i++) r.pressKey("alex", 0x2A, 0, "\b");
            }, "alex: End, then Backspace to clear the field");
            typeOn(b, "alex", s[0]);
            b.await(r -> shows(r, "alex", s[1]), "alex: '" + s[0] + "' finds " + s[1], 10_000);
            b.await(r -> click(r, "alex", "+"), "alex: click + on " + s[1], 5_000);
            b.await(r -> click(r, "alex", "Road Trip"), "alex: click Road Trip", 5_000);
            b.await(r -> shows(r, "alex", "Added " + s[1] + " to Road Trip"), "alex: 'Added " + s[1] + " to Road Trip'", 10_000);
        }
        b.note("alex: open Road Trip and move Village Fiber down (Data Center Nights first)");
        b.await(r -> click(r, "alex", "Library"), "alex: click Library", 5_000);
        b.await(r -> click(r, "alex", "Road Trip"), "alex: click Road Trip", 5_000);
        b.await(r -> shows(r, "alex", "private"), "Road Trip is private", 10_000);
        b.await(r -> click(r, "alex", "v"), "alex: click v on the first song (move it down)", 5_000);
        b.mutate(r -> r.typeLine("server", "cat /var/music/store"), "server: cat /var/music/store");
        b.expect("server", "playlist\\s+p1\\s+alex\\s+0\\s+\\d+\\s+\\d+\\s+Road Trip\\s+\\S+\\s+\\S+", "musicd keeps Road Trip (alex's, private, two songs)");

        b.note("Control: bo, with an account of their own, can't see Road Trip");
        openMusic(b, "bo", "10.0.9.1");
        b.await(r -> click(r, "bo", "Sign in"), "bo: click Sign in", 10_000);
        signUp(b, "bo", "bo");
        b.await(r -> click(r, "bo", "Library"), "bo: click Library", 5_000);
        b.await(r -> shows(r, "bo", "Liked Songs"), "bo's Library", 10_000);
        b.await(r -> lacks(r, "bo", "Road Trip"), "control: no Road Trip in bo's Library", 2_000);
        search(b, "bo", "road trip", "Nothing found");
        b.note("bo signs out and signs in as alex");
        b.await(r -> click(r, "bo", "Settings"), "bo: click Settings", 5_000);
        b.await(r -> click(r, "bo", "Sign out of this server"), "bo: click Sign out of this server", 5_000);
        b.await(r -> click(r, "bo", "Sign in"), "bo: click Sign in", 10_000);
        signIn(b, "bo", "alex");
        b.await(r -> click(r, "bo", "Library"), "bo: click Library", 5_000);
        b.await(r -> click(r, "bo", "Road Trip"), "bo: alex's Road Trip is there: click it", 10_000);
        b.await(r -> {
            var c = comp(r, "bo");
            for (var w : c.windows()) {
                if (!w.appId().equals(MUSIC) || !w.mapped()) continue;
                var t = c.textsOf(w.id());
                int fiber = t.indexOf("Village Fiber"), nights = t.indexOf("Data Center Nights");
                return fiber >= 0 && nights >= 0 && nights < fiber ? null : "texts " + t;
            }
            return "no window";
        }, "in alex's order: Data Center Nights first, then Village Fiber (moved down)", 10_000);
        b.await(r -> click(r, "bo", "Play"), "bo: click Play", 5_000);
        b.await(r -> allowAll(r, "bo", x -> drained(x, BO_SPEAKER) > 22_050 ? null : "speaker " + drained(x, BO_SPEAKER)),
                "bo: allow Music the speaker; the playlist plays on bo's Speaker", 25_000);
        b.await(r -> shows(r, "bo", "Data Center Nights"), "it starts with Data Center Nights", 5_000);
        return b.timeLimit(240_000).build();
    }


    // ------------------------------------------------------------------ music_background (an ePhone)

    static final String SSID = "ecm-radio", GW_IP = "192.168.79.1", PHONE_IP = "192.168.79.2";
    static final BlockPos AP = new BlockPos(3, 1, 1), GW = new BlockPos(3, 1, 4), CHEST = new BlockPos(1, 1, 1);
    static final List<BlockPos> CABLE = List.of(new BlockPos(3, 0, 4), new BlockPos(3, 0, 3), new BlockPos(3, 0, 2), new BlockPos(3, 0, 1));
    static final int HID_H = 0x0B, HID_N = 0x11;

    private static long phoneSamples(ScenarioRun r) {
        return PhoneGuiScenarios.host(r).speakerAudio().samplesDrained();
    }

    /** Tap {@code text} in the phone's Music window. */
    private static String tapMusic(ScenarioRun r, String text) {
        Compositor c = PhoneGuiScenarios.comp(r);
        Compositor.WindowInfo w = PhoneGuiScenarios.onScreen(r, MUSIC);
        if (c == null || w == null) return "Music isn't on screen";
        int[] at = c.locateText(w.id(), text, false);
        if (at == null) return "\"" + text + "\" not in Music: " + c.textsOf(w.id());
        PhoneGuiScenarios.tap(r, at);
        return null;
    }

    private static String phoneShows(ScenarioRun r, String text) {
        Compositor c = PhoneGuiScenarios.comp(r);
        Compositor.WindowInfo w = PhoneGuiScenarios.onScreen(r, MUSIC);
        if (c == null || w == null) return "Music isn't on screen";
        return c.textsOf(w.id()).stream().anyMatch(t -> t.contains(text)) ? null : "Music shows " + c.textsOf(w.id());
    }

    /** Tap Allow on any permission the phone asks for while {@code done} doesn't hold yet. */
    private static String allowOnPhone(ScenarioRun r, java.util.function.Function<ScenarioRun, String> done) {
        Compositor c = PhoneGuiScenarios.comp(r);
        List<String> t = c.layerTexts("overlay");
        if (t.contains("Allow") && String.join(" ", t).contains("Music")) {
            PhoneGuiScenarios.tap(r, c.locateLayerText("overlay", "Allow", false));
            return "tapped Allow on " + t;
        }
        return done.apply(r);
    }

    /** The phone's speaker plays: at least a quarter of 22050 samples a second over {@code ms}. */
    private static void phonePlays(Scenario.Builder b, int ms, String what) {
        b.mutate(r -> {
            st(r).samples = phoneSamples(r);
            st(r).at = System.currentTimeMillis();
        }, "count the phone speaker's samples");
        b.waitMs(ms, "listen for " + ms / 1000.0 + " s");
        b.await(r -> {
            long got = phoneSamples(r) - st(r).samples;
            long want = (System.currentTimeMillis() - st(r).at) * 22050 / 4000;
            r.say("§f  " + got + " samples in " + (System.currentTimeMillis() - st(r).at) + " ms");
            if (got < want) r.fail("the music stopped: " + got + " samples, wanted " + want);
            return null;
        }, what, 1_000);
    }

    private static void phoneSilent(Scenario.Builder b, String what) {
        b.waitMs(1_000, "what was queued plays out");
        b.mutate(r -> st(r).samples = phoneSamples(r), "count the phone speaker's samples");
        b.waitMs(2_000, "listen for 2 s");
        b.await(r -> {
            long got = phoneSamples(r) - st(r).samples;
            if (got > 1_000) r.fail("still playing: " + got + " samples");
            return null;
        }, what, 1_000);
    }

    private static Scenario background() {
        List<BlockPos> floor = new java.util.ArrayList<>(PlayerKit.box(-2, 0, -2, 6, 0, 6));
        floor.removeAll(CABLE);
        List<BlockPos> foot = new java.util.ArrayList<>(floor);
        foot.addAll(CABLE);
        foot.add(AP);
        var b = Scenario.builder("music_background", "the ePhone gets Music over Wi-Fi, signs in and plays; it keeps playing at home,"
                        + " in the hotbar and in the pocket, with a Now Playing card; hotkey 1 pauses; in a chest it stops")
                .asPlayer()
                .realTime()
                .host("server", GW, GW_IP + "/24")
                .decor(PlayerKit.decor(foot, r -> PlayerKit.fill(r, floor, net.minecraft.world.level.block.Blocks.SMOOTH_STONE.defaultBlockState()), r -> {
                    for (BlockPos p : CABLE) r.player().place(com.example.evanscomputermod.block.ModBlocks.NETWORK_CABLE.get(), r.abs(p), Direction.UP);
                    PlayerKit.accessPoint(r, AP, Direction.WEST, SSID, null, 6, 0);
                }, PhoneScenarios::cleanup))
                .note("Setup (done for you): a server computer cabled to an open Access Point (" + SSID + ")");
        b.send("server", "ifconfig eth0 " + GW_IP + "/24");
        b.expect("server", "eth0: inet " + GW_IP.replace(".", "\\.") + "/24", "the server has " + GW_IP);
        server(b);
        b.note("A player takes a new ePhone and holds it");
        b.mutate(r -> {
            PhoneScenarios.select(PhoneScenarios.holder(r), 0);
            PhoneScenarios.givePhone(r);
            com.example.evanscomputermod.phone.PhoneConfig.override("battery.enabled", false);
        }, "select an empty hotbar slot, take a new phone and hold it (batteries off for the run)");
        b.await(PhoneGuiScenarios::guiBooted, "the phone boots into its shell", 40_000);
        PhoneScenarios.openTerminal(b);
        b.mutate(r -> PhoneScenarios.type(r, PhoneScenarios.holder(r), "wifi connect " + SSID + "\n"), "in the phone's Terminal: wifi connect " + SSID);
        b.await(r -> {
            String s = PhoneScenarios.screen(r);
            return s.stripTrailing().endsWith("/ >") && (s.contains(SSID) || s.contains("address by hand")) ? null : s;
        }, "the phone joins " + SSID, 40_000);
        PhoneScenarios.phoneRun(b, "ifconfig wlan0 " + PHONE_IP + "/24", "wlan0: inet " + PHONE_IP.replace(".", "\\."), null, "the phone has " + PHONE_IP, 10_000);
        PhoneScenarios.phoneRun(b, "pkg repo add lab http://" + GW_IP + ":8080/", "Added lab", "^pkg: ", "the phone adds the server's repository", 20_000);
        PhoneScenarios.phoneRun(b, "pkg install " + MUSIC, "Installed Music 1\\.0\\.0", "^pkg: ", "the phone installs Music over Wi-Fi", 30_000);
        PhoneGuiScenarios.openApp(b, "Music", MUSIC);
        b.await(r -> allowOnPhone(r, x -> phoneShows(x, "Village Radio")), "Music asks for the network (Allow) and finds Village Radio", 30_000);
        b.note("Sign in: Create account on the shared sign-in screen");
        b.await(r -> tapMusic(r, "Sign in"), "tap Sign in", 10_000);
        b.await(r -> phoneShows(r, "First time here"), "first contact: the server's key", 20_000);
        b.await(r -> tapMusic(r, "No account yet? Create one"), "tap \"No account yet? Create one\"", 5_000);
        b.await(r -> tapMusic(r, "Username"), "tap Username", 5_000);
        b.mutate(r -> PhoneGuiScenarios.type(r, "alex"), "type alex");
        b.await(r -> tapMusic(r, "Password"), "tap Password", 5_000);
        b.mutate(r -> PhoneGuiScenarios.type(r, DiscordScenarios.PASSWORD), "type the password");
        b.await(r -> tapMusic(r, "Password again"), "tap Password again", 5_000);
        b.mutate(r -> PhoneGuiScenarios.type(r, DiscordScenarios.PASSWORD), "type it again");
        b.await(r -> tapMusic(r, "Create account"), "tap Create account", 5_000);
        b.await(r -> phoneShows(r, "Signed in as"), "signed in", 30_000);
        b.await(r -> tapMusic(r, "Done"), "tap Done", 5_000);
        b.await(r -> tapMusic(r, "Search"), "tap Search", 5_000);
        b.await(r -> tapMusic(r, "Songs, artists, albums, playlists"), "tap the search field", 5_000);
        b.mutate(r -> PhoneGuiScenarios.type(r, "redst"), "type redst");
        b.await(r -> phoneShows(r, "Redstone Sunrise"), "'redst' finds Redstone Sunrise", 10_000);
        b.await(r -> tapMusic(r, "Redstone Sunrise"), "tap Redstone Sunrise", 5_000);
        b.await(r -> allowOnPhone(r, x -> phoneSamples(x) > 22_050 ? null : "samples " + phoneSamples(x)),
                "Music asks for the speaker (Allow); the phone's speaker plays", 30_000);
        b.note("Home: the first-time background prompt");
        b.mutate(r -> PhoneGuiScenarios.altKey(r, HID_H), "Alt+H: home");
        b.await(r -> {
            List<String> t = PhoneGuiScenarios.comp(r).layerTexts("overlay");
            return String.join(" ", t).contains("Keep Music running in the background") && t.contains("Allow") ? null : "overlay " + t;
        }, "the phone asks: Keep Music running in the background for audio?", 15_000);
        PhoneGuiScenarios.tapShell(b, "overlay", "Allow", "tap Allow");
        b.await(r -> PhoneGuiScenarios.comp(r).layerTexts("status").contains("AUDIO") ? null : "status " + PhoneGuiScenarios.comp(r).layerTexts("status"),
                "the status bar shows AUDIO (Music plays in the background)", 10_000);
        b.waitMs(1_500, "Music picks up again after the question (it was held while the phone asked)");
        phonePlays(b, 2_000, "at home Music keeps playing");
        b.mutate(r -> PhoneGuiScenarios.altKey(r, HID_N), "Alt+N: the notification sheet");
        b.await(r -> PhoneGuiScenarios.comp(r).layerTexts("overlay").stream().anyMatch(t -> t.contains("Redstone Sunrise")) ? null
                : "overlay " + PhoneGuiScenarios.comp(r).layerTexts("overlay"), "the sheet has a Now Playing card: Redstone Sunrise", 10_000);
        b.mutate(r -> PhoneGuiScenarios.altKey(r, HID_N), "Alt+N: close the sheet");
        b.mutate(r -> PhoneScenarios.select(PhoneScenarios.holder(r), 1), "select hotbar slot 2 (the phone in the hotbar)");
        b.await(r -> PhoneGuiScenarios.host(r).state() == com.example.evanscomputermod.phone.PhoneState.ASLEEP ? null
                : "phone " + PhoneGuiScenarios.host(r).state(), "asleep in the hotbar", 5_000);
        phonePlays(b, 2_000, "in the hotbar Music keeps playing");
        b.mutate(r -> PhoneScenarios.move(r, PhoneScenarios.holder(r), 9), "move the phone into the main inventory (the pocket)");
        b.await(r -> PhoneGuiScenarios.host(r).state() == com.example.evanscomputermod.phone.PhoneState.ASLEEP && PhoneGuiScenarios.host(r).isAttached()
                ? null : "phone " + PhoneGuiScenarios.host(r).state(), "in the pocket: asleep, radios on", 5_000);
        phonePlays(b, 3_000, "in the pocket Music keeps playing");
        b.note("Hotkey 1 (Play/Pause), with the phone back in hand");
        b.mutate(r -> {
            PhoneScenarios.move(r, PhoneScenarios.holder(r), 0);
            PhoneScenarios.select(PhoneScenarios.holder(r), 0);
        }, "move the phone back to hotbar slot 1 and hold it");
        b.await(r -> PhoneGuiScenarios.host(r).state() == com.example.evanscomputermod.phone.PhoneState.AWAKE ? null
                : "phone " + PhoneGuiScenarios.host(r).state(), "the phone is awake in hand", 5_000);
        b.note("(Hotkeys go to the app on screen: the shell routes them to the focused window.)");
        PhoneGuiScenarios.openApp(b, "Music", MUSIC);
        b.mutate(r -> PhoneGuiScenarios.hotkey(r, 1), "press hotkey slot 1 (Play/Pause)");
        phoneSilent(b, "paused: the speaker is quiet");
        b.mutate(r -> PhoneGuiScenarios.hotkey(r, 1), "press hotkey slot 1 again");
        phonePlays(b, 2_000, "playing again");
        b.note("Control: put the phone in a chest");
        b.mutate(r -> PhoneScenarios.move(r, PhoneScenarios.holder(r), 9), "the phone into the main inventory again");
        b.mutate(r -> {
            r.player().place(net.minecraft.world.level.block.Blocks.CHEST, r.abs(CHEST), Direction.NORTH);
            r.player().useEmpty(r.abs(CHEST), Direction.UP, false);
            r.player().shiftClick(27);
            r.player().closeScreen();
        }, "place a chest, open it, shift-click the phone in and close it");
        b.await(r -> PhoneGuiScenarios.host(r).state() == com.example.evanscomputermod.phone.PhoneState.OFF ? null
                : "phone " + PhoneGuiScenarios.host(r).state(), "in the chest the phone is off", 5_000);
        phoneSilent(b, "the music stops");
        return b.timeLimit(180_000).build();
    }
    // ------------------------------------------------------------------ music_duck

    /** Music's stream on the Speaker at {@code speaker} (by the program that opened it), or null. */
    static com.example.evanscomputermod.speaker.AudioStream musicStream(ScenarioRun r, String host, BlockPos speaker) {
        if (!(r.level().getBlockEntity(r.abs(speaker)) instanceof SpeakerBlockEntity s)) return null;
        var apps = AppScenarios.computer(r, host).getAppManager();
        for (var st : s.audio().streams()) {
            var a = apps.instanceOf(st.pid());
            if (a != null && a.appId().equals(MUSIC) && !st.isClosed()) return st;
        }
        return null;
    }

    private static final Map<ScenarioRun, long[]> QUIET = java.util.Collections.synchronizedMap(new WeakHashMap<>());

    private static Scenario duck() {
        var b = DiscordScenarios.threeComputers("music_duck", "bo listens to Music and is in a Discord voice call: the music is lowered"
                + " while alex talks in the call and comes back after; while the call is quiet it isn't lowered");
        BlockPos alexMic = DiscordScenarios.ALEX_MIC, serverSpeaker = DiscordScenarios.SERVER_SPEAKER;
        b.decor(PlayerKit.decor(List.of(alexMic, serverSpeaker, BO_SPEAKER, DiscordScenarios.ALEX_SPEAKER), null, r -> {
            ScenarioPlayer p = r.player();
            p.place(com.example.evanscomputermod.voice.VoiceContent.MICROPHONE.get(), r.abs(alexMic), Direction.NORTH,
                    s -> s.getValue(com.example.evanscomputermod.voice.MicrophoneBlock.FACING) == Direction.NORTH);
            for (BlockPos sp : List.of(serverSpeaker, BO_SPEAKER, DiscordScenarios.ALEX_SPEAKER)) {
                p.place(SpeakerContent.SPEAKER.get(), r.abs(sp), Direction.NORTH, s -> s.getValue(SpeakerBlock.FACING) == Direction.NORTH);
            }
        }, null));
        b.note("Also placed for you: a Microphone west and a Speaker east of alex, a Speaker of the server two blocks from that Microphone (alex's voice: the"
                + " server beeps), and a Speaker beside bo (no Microphone: bo only listens in the call, so the music isn't sent back into it).");
        b.realTime();
        b.note("server: discordd, musicd with the three songs, disco serve; both packages in one repository");
        b.mutate(r -> PlayerKit.dropFiles(r, "server", "srv/music", SONG_DIR, SONGS),
                "copy the three Tech Village songs into the server's /srv/music [exception: binary files copied in; docs/SCENARIO_EXCEPTIONS.md]");
        b.send("server", "discordd &");
        b.expect("server", "discordd listening on port 7777", "discordd listens");
        b.send("server", "musicd --name \"Village Radio\" &");
        b.expect("server", "scan: 3 songs \\(3 new", "musicd scans its three songs");
        b.send("server", "disco serve &");
        b.expect("server", "disco: answering on UDP port 5354", "disco serve answers");
        for (String[] p : new String[][]{{"discord", "Discord", "dev\\.ecm\\.discord"}, {"music", "Music", "dev\\.ecm\\.music"}}) {
            b.send("server", p[0] + " --source /src/" + p[0]);
            b.expect("server", p[0] + ": wrote " + p[1] + " \\(" + p[2] + "\\) 1\\.0\\.0 to /src/" + p[0], p[1] + "'s package folder", "^" + p[0] + ": can't");
            b.expect("server", PlayerKit.PROMPT, "shell");
            b.send("server", "pkg build /src/" + p[0] + " /srv/repo");
            b.expect("server", "^Built /srv/repo/" + p[2] + "-1\\.0\\.0\\.ecmpkg", "pkg build packs " + p[1], "^pkg: ");
            b.expect("server", PlayerKit.PROMPT, "shell");
        }
        b.send("server", "pkg serve /srv/repo 8080 &");
        b.expect("server", "Serving 2 packages from /srv/repo on port 8080", "the repository is online with both", "^pkg: ");

        b.note("alex: Discord, into the General voice channel");
        DiscordScenarios.openDiscord(b, "alex", "10.0.8.1");
        DiscordScenarios.signUp(b, "alex", "alex");
        b.await(r -> DiscordScenarios.shows(r, "alex", "General"), "alex: the home server, with its General voice channel", 25_000);
        DiscordScenarios.joinVoice(b, "alex");

        b.note("bo: Music and Discord from the repository");
        b.send("bo", "pkg repo add lab http://10.0.9.1:8080/");
        b.expect("bo", "lab", "the repository is added", "^pkg: ");
        b.expect("bo", PlayerKit.PROMPT, "shell");
        for (String[] p : new String[][]{{MUSIC, "Music"}, {DiscordScenarios.DISCORD, "Discord"}}) {
            b.send("bo", "pkg install " + p[0]);
            b.expect("bo", "^Installed " + p[1] + " 1\\.0\\.0", "bo installs " + p[1], "^pkg: ");
            b.expect("bo", PlayerKit.PROMPT, "shell");
        }
        b.send("bo", "gui");
        b.expect("bo", "^gui: started", "the desktop starts", "(?i)unknown command|no GUI device");
        b.await(r -> comp(r, "bo") != null && comp(r, "bo").shellConnected() ? null : "waiting for the shell", "bo's desktop is up", 15_000);
        b.await(r -> DiscordScenarios.clickOn(r, "bo", "taskbar", "Apps"), "bo: click Apps on the taskbar", 10_000);
        b.await(r -> DiscordScenarios.clickOn(r, "bo", "popup", "Music"), "bo: click Music in the launcher", 5_000);
        b.await(r -> allowAll(r, "bo", x -> shows(x, "bo", "Village Radio")), "bo: allow Music the network; it finds Village Radio", 30_000);
        b.await(r -> click(r, "bo", "Sign in"), "bo: click Sign in", 10_000);
        signUp(b, "bo", "bo");
        search(b, "bo", "redst", "Redstone Sunrise");
        b.await(r -> click(r, "bo", "Redstone Sunrise"), "bo: click Redstone Sunrise", 5_000);
        b.await(r -> allowAll(r, "bo", x -> musicStream(x, "bo", BO_SPEAKER) != null && drained(x, BO_SPEAKER) > 22_050 ? null
                        : "speaker " + drained(x, BO_SPEAKER) + "; " + shows(x, "bo", "\u0000") + "; permission " + comp(x, "bo").layerTexts("permission")),
                "bo: allow Music the speaker; Redstone Sunrise plays on bo's Speaker", 25_000);
        b.await(r -> DiscordScenarios.clickOn(r, "bo", "taskbar", "Apps"), "bo: click Apps", 10_000);
        b.await(r -> DiscordScenarios.clickOn(r, "bo", "popup", "Discord"), "bo: click Discord in the launcher", 5_000);
        b.await(r -> comp(r, "bo").layerTexts("permission").contains("Allow Discord to use network?") ? null
                : "no dialog: " + comp(r, "bo").layerTexts("permission"), "bo: Discord asks for the network", 20_000);
        b.await(r -> DiscordScenarios.clickOn(r, "bo", "permission", "Allow"), "bo: click Allow", 5_000);
        DiscordScenarios.signUp(b, "bo", "bo");
        b.await(r -> DiscordScenarios.shows(r, "bo", "General"), "bo: the home server, with its General voice channel", 25_000);
        DiscordScenarios.joinVoice(b, "bo");

        b.note("Control: the call is quiet: the music is not lowered");
        b.mutate(r -> QUIET.put(r, new long[]{System.currentTimeMillis()}), "start listening");
        b.await(r -> {
            var s = musicStream(r, "bo", BO_SPEAKER);
            if (s == null) return "no Music stream on bo's Speaker";
            if (s.ducked()) {
                r.fail("Music was lowered while nobody talked");
                return null;
            }
            return System.currentTimeMillis() - QUIET.get(r)[0] >= 3_000 ? null : "listening";
        }, "control: for three seconds of a quiet call Music plays at full volume", 6_000);
        b.note("alex talks (the server's Speaker beeps beside alex's Microphone): bo's music is lowered");
        b.send("server", "beep 700 4000 --wave sine &");
        b.await(r -> {
            var s = musicStream(r, "bo", BO_SPEAKER);
            return s != null && s.ducked() ? null : "music stream " + s + "; bo's mixer: " + AppScenarios.computer(r, "bo").volumeMixer().text().replace('\n', ';')
                    + "; alex's mixer: " + AppScenarios.computer(r, "alex").volumeMixer().text().replace('\n', ';') + "; alex: " + DiscordScenarios.shows(r, "alex", "\u0000") + "; bo: " + DiscordScenarios.shows(r, "bo", "\u0000");
        }, "while alex's voice reaches bo, Music's stream is ducked", 10_000);
        b.await(r -> {
            var s = musicStream(r, "bo", BO_SPEAKER);
            return s != null && !s.ducked() ? null : "music stream " + s;
        }, "alex stops (the beep ends): Music comes back to full volume", 15_000);
        b.await(r -> drained(r, BO_SPEAKER) > 0 && musicStream(r, "bo", BO_SPEAKER) != null ? null : "no music", "Music is still playing", 2_000);
        return b.timeLimit(300_000).build();
    }
    static {
        for (Scenario s : List.of(stream(), playlist(), background(), duck())) ALL.put(s.name, s);
    }
}
//?}
