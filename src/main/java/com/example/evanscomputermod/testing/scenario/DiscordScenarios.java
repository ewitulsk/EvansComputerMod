package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.ui.Compositor;
import com.example.evanscomputermod.speaker.SpeakerBlock;
import com.example.evanscomputermod.speaker.SpeakerBlockEntity;
import com.example.evanscomputermod.speaker.SpeakerContent;
import com.example.evanscomputermod.voice.MicrophoneBlock;
import com.example.evanscomputermod.voice.VoiceContent;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Discord (docs/DISCORD.md), spawnable with {@code /ecm scenario spawn <name>}; also the
 * {@code ecm_discord} GameTests. (The single-computer flows, installing Discord from the
 * App Store and finding a backend on the network, are {@code discord_app} and
 * {@code service_discovery} in {@link AppScenarios}.)
 *
 * <p>{@code discord_servers}: computer {@code server} runs two Discord backends
 * ({@code discordd} on 7777 named Lab and on 7790 named Village, each with its own data
 * folder) and {@code disco serve}; {@code alex} is cabled to its bottom (LAN 10.0.8.0/24),
 * {@code bo} to its top (LAN 10.0.9.0/24). Both install Discord with {@code pkg} and open
 * it in the GUI: each finds both backends by itself and shows both home servers on its
 * rail. alex makes a server, "Builders", hosted on Village, with a text channel
 * ("Build Plans" becomes #build-plans) and a voice channel (Lounge). Controls: bo's rail
 * doesn't have Builders and bo can't make channels in it; then bo joins it from "Join a
 * Server", posts in #build-plans, and alex reads it. The backend kept it on disk.
 * <pre>
 *   server (0,1,0); alex (4,1,0) under-cabled; bo (0,1,30) over-cabled.
 * </pre>
 *
 * <p>{@code discord_voice}: the same three computers, one backend. alex and bo each
 * have a Microphone and a Speaker next to their computer; the server has a Speaker two
 * blocks from alex's Microphone. Both open Discord and join the home server's General
 * voice channel (allowing it the microphone). Control: nothing plays on bo's Speaker
 * while the room is quiet. The server's Speaker beeps beside alex: alex's microphone
 * hears it, Discord sends it over UDP, the backend forwards it, and bo's Speaker plays
 * it (bo sees alex speaking). Control: alex mutes and bo's Speaker goes quiet.
 */
public final class DiscordScenarios {

    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static final String DISCORD = AppScenarios.DISCORD;
    static final BlockPos SERVER = new BlockPos(0, 1, 0), ALEX = new BlockPos(4, 1, 0), BO = new BlockPos(0, 1, 30);
    static final BlockPos ALEX_MIC = new BlockPos(3, 1, 0), ALEX_SPEAKER = new BlockPos(5, 1, 0);
    static final BlockPos BO_MIC = new BlockPos(1, 1, 30), BO_SPEAKER = new BlockPos(-1, 1, 30);
    static final BlockPos SERVER_SPEAKER = new BlockPos(1, 1, 0);

    static {
        for (Scenario s : List.of(servers(), voice())) ALL.put(s.name, s);
    }

    private DiscordScenarios() {}

    // ------------------------------------------------------------------ GUI on a named computer

    static Compositor comp(ScenarioRun r, String host) {
        var c = AppScenarios.computer(r, host);
        return c == null || c.getUiManager() == null ? null : c.getUiManager().compositor();
    }

    static String click(ScenarioRun r, String host, String title, String text) {
        var c = comp(r, host);
        if (c == null) return "no GUI on " + host;
        int[] at = c.findText(title, text);
        if (at == null) return "\"" + text + "\" not shown in " + title + " on " + host + " yet";
        r.click(host, at[0], at[1]);
        return null;
    }

    static String clickOn(ScenarioRun r, String host, String layer, String text) {
        var c = comp(r, host);
        if (c == null) return "no GUI on " + host;
        int[] at = layer.equals("popup") ? c.locatePopupText(text, false) : c.locateLayerText(layer, text, false);
        if (at == null) return "\"" + text + "\" not shown on " + host + "'s " + layer + " yet";
        r.click(host, at[0], at[1]);
        return null;
    }

    /** Null once {@code host}'s Discord window shows a text containing {@code text}. */
    static String shows(ScenarioRun r, String host, String text) {
        var c = comp(r, host);
        if (c == null) return "no GUI on " + host;
        for (var w : c.windows()) {
            if (!w.appId().equals(DISCORD) || !w.mapped()) continue;
            var texts = c.textsOf(w.id());
            return texts.stream().anyMatch(t -> t.contains(text)) ? null : host + "'s Discord shows " + texts;
        }
        return "no Discord window on " + host;
    }

    static String lacks(ScenarioRun r, String host, String text) {
        return shows(r, host, text) == null ? host + "'s Discord shows " + text : null;
    }

    static void typeOn(Scenario.Builder b, String host, String text) {
        b.mutate(r -> r.typeText(host, text), host + ": type '" + text + "'");
    }

    /** {@code host}: install Discord from the server's repository with pkg, start the GUI, open Discord, allow it the network. */
    static void openDiscord(Scenario.Builder b, String host, String repoIp) {
        b.send(host, "pkg repo add lab http://" + repoIp + ":8080/");
        b.expect(host, "lab", "the repository is added", "^pkg: ");
        b.expect(host, PlayerKit.PROMPT, "shell");
        b.send(host, "pkg install " + DISCORD);
        b.expect(host, "^Installed Discord 1\\.0\\.0 \\(dev\\.ecm\\.discord\\)", host + " installs Discord over the network", "^pkg: ");
        b.expect(host, PlayerKit.PROMPT, "shell");
        b.send(host, "gui");
        b.expect(host, "^gui: started", "the desktop starts", "(?i)unknown command|no GUI device");
        b.await(r -> comp(r, host) != null && comp(r, host).shellConnected() ? null : "waiting for the shell", host + "'s desktop is up", 15_000);
        b.await(r -> clickOn(r, host, "taskbar", "Apps"), host + ": click Apps on the taskbar", 10_000);
        b.await(r -> clickOn(r, host, "popup", "Discord"), host + ": click Discord in the launcher", 5_000);
        b.await(r -> comp(r, host).layerTexts("permission").contains("Allow Discord to use network?") ? null
                : "no dialog: " + comp(r, host).layerTexts("permission"), host + ": Discord asks for the network as it looks for backends", 20_000);
        b.await(r -> clickOn(r, host, "permission", "Allow"), host + ": click Allow", 5_000);
    }

    static Scenario.Builder threeComputers(String name, String what) {
        var b = Scenario.builder(name, what).asPlayer();
        b.host("server", SERVER, "-");
        b.host("alex", ALEX, "-");
        b.host("bo", BO, "-");
        b.link("lan_a", "server", Direction.DOWN, "alex", Direction.DOWN, Scenario.Path.from(new BlockPos(0, 0, 0)).go(Direction.EAST, 4));
        b.link("lan_b", "server", Direction.UP, "bo", Direction.UP, Scenario.Path.from(new BlockPos(0, 2, 0)).go(Direction.SOUTH, 30));
        b.note("Setup (done for you): a server cabled to alex below (eth0) and to bo above (eth1, 30 blocks south).");
        b.send("server", "ifconfig eth0 10.0.8.1/24");
        b.expect("server", "eth0: inet 10\\.0\\.8\\.1/24", "server has 10.0.8.1 on alex's LAN");
        b.send("server", "ifconfig eth1 10.0.9.1/24");
        b.expect("server", "eth1: inet 10\\.0\\.9\\.1/24", "server has 10.0.9.1 on bo's LAN");
        b.send("alex", "ifconfig eth0 10.0.8.2/24");
        b.expect("alex", "eth0: inet 10\\.0\\.8\\.2/24", "alex has 10.0.8.2");
        b.send("bo", "ifconfig eth1 10.0.9.2/24");
        b.expect("bo", "eth1: inet 10\\.0\\.9\\.2/24", "bo has 10.0.9.2");
        return b;
    }

    /** The server: the Discord package in a repository (built from the server's own discord program). */
    static void repository(Scenario.Builder b) {
        b.send("server", "discord --source /src/discord");
        b.expect("server", "discord: wrote Discord \\(dev\\.ecm\\.discord\\) 1\\.0\\.0 to /src/discord", "Discord's package folder", "^discord: can't");
        b.expect("server", PlayerKit.PROMPT, "shell");
        b.send("server", "pkg build /src/discord /srv/repo");
        b.expect("server", "^Built /srv/repo/dev\\.ecm\\.discord-1\\.0\\.0\\.ecmpkg", "pkg build packs it", "^pkg: ");
        b.expect("server", PlayerKit.PROMPT, "shell");
        b.send("server", "pkg serve /srv/repo 8080 &");
        b.expect("server", "Serving 1 packages from /srv/repo on port 8080", "the repository is online", "^pkg: ");
    }

    // ------------------------------------------------------------------ discord_servers

    private static Scenario servers() {
        var b = threeComputers("discord_servers",
                "two Discord backends on one computer; two players find both by themselves, alex makes a server with a text and a"
                        + " voice channel on one of them, bo joins it and they talk in its #build-plans");
        b.note("server: two Discord backends (Lab on 7777, Village on 7790), disco serve, the Discord package");
        b.send("server", "discordd 7777 --name Lab --data /var/lab &");
        b.expect("server", "discordd listening on port 7777 \\(voice on UDP 7778\\), \"Lab\"", "backend Lab listening");
        b.send("server", "discordd 7790 --name Village --data /var/village &");
        b.expect("server", "discordd listening on port 7790 \\(voice on UDP 7791\\), \"Village\"", "backend Village listening");
        b.send("server", "disco serve &");
        b.expect("server", "disco: answering on UDP port 5354 .* for 4 services", "disco serve answers for both backends (as discord and chat)");
        repository(b);

        b.note("alex: Discord finds both backends; their home servers side by side on the rail");
        openDiscord(b, "alex", "10.0.8.1");
        b.await(r -> shows(r, "alex", "LA") == null && shows(r, "alex", "VI") == null ? null : shows(r, "alex", "VI"),
                "alex's rail: Lab (LA) and Village (VI), found by themselves", 25_000);
        b.expect("server", "joined from 10\\.0\\.8\\.2", "a backend sees alex join");

        b.note("alex: + (Add a server), Create My Own, a name, host it on Village, Create");
        b.await(r -> click(r, "alex", "Discord", "+"), "alex: click +", 5_000);
        b.await(r -> click(r, "alex", "Discord", "Create My Own"), "alex: click Create My Own", 5_000);
        b.await(r -> click(r, "alex", "Discord", "My Server"), "alex: click the name field", 5_000);
        typeOn(b, "alex", "Builders");
        b.await(r -> click(r, "alex", "Discord", "Host on Village"), "alex: host it on Village", 5_000);
        b.await(r -> click(r, "alex", "Discord", "Create"), "alex: click Create", 5_000);
        b.expect("server", "made the server \"Builders\"", "Village makes the server Builders");
        b.await(r -> shows(r, "alex", "Builders"), "Builders opens on alex's Discord (owner)", 10_000);

        b.note("alex (its owner): a text channel and a voice channel");
        b.await(r -> click(r, "alex", "Discord", "+ Channel"), "alex: click + Channel", 5_000);
        b.await(r -> click(r, "alex", "Discord", "new-channel"), "alex: click the channel name field", 5_000);
        typeOn(b, "alex", "Build Plans");
        b.await(r -> click(r, "alex", "Discord", "Create Channel"), "alex: click Create Channel", 5_000);
        b.await(r -> shows(r, "alex", "# build-plans"), "#build-plans (the name made Discord-style)", 10_000);
        b.await(r -> click(r, "alex", "Discord", "+ Voice"), "alex: click + Voice", 5_000);
        b.await(r -> click(r, "alex", "Discord", "New Voice"), "alex: click the channel name field", 5_000);
        typeOn(b, "alex", "Lounge");
        b.await(r -> click(r, "alex", "Discord", "Create Channel"), "alex: click Create Channel", 5_000);
        b.await(r -> shows(r, "alex", "Lounge"), "the voice channel Lounge", 10_000);

        b.note("bo: Discord finds both backends too; Builders isn't on bo's rail until bo joins it");
        openDiscord(b, "bo", "10.0.9.1");
        b.await(r -> shows(r, "bo", "VI"), "bo's rail has the Village home server", 25_000);
        b.await(r -> lacks(r, "bo", "BU"), "control: Builders is not on bo's rail (bo hasn't joined)", 2_000);
        b.await(r -> click(r, "bo", "Discord", "+"), "bo: click +", 5_000);
        b.await(r -> click(r, "bo", "Discord", "Join a Server"), "bo: click Join a Server", 5_000);
        b.await(r -> shows(r, "bo", "Builders"), "bo sees Builders (1 member, on Village)", 10_000);
        b.await(r -> click(r, "bo", "Discord", "Join"), "bo: click Join", 5_000);
        b.expect("server", "bo-?\\w* joined the server \"Builders\"|joined the server \"Builders\"", "Village: bo joined Builders");
        b.await(r -> shows(r, "bo", "# build-plans"), "Builders opens on bo's Discord with #build-plans", 10_000);
        b.await(r -> lacks(r, "bo", "+ Channel"), "control: bo doesn't own Builders, so no + Channel", 2_000);

        b.note("bo posts in #build-plans; alex reads it");
        b.await(r -> click(r, "bo", "Discord", "# build-plans"), "bo: click # build-plans", 5_000);
        b.await(r -> click(r, "bo", "Discord", "Message #build-plans"), "bo: click the message box", 5_000);
        typeOn(b, "bo", "a tower by the lake?");
        b.await(r -> click(r, "bo", "Discord", "Send"), "bo: click Send", 5_000);
        b.expect("server", "#build-plans <\\S+> a tower by the lake\\?", "Village keeps the message");
        b.await(r -> click(r, "alex", "Discord", "# build-plans"), "alex: click # build-plans", 5_000);
        b.await(r -> shows(r, "alex", "a tower by the lake?"), "alex reads bo's message", 10_000);
        b.send("server", "cat /var/village/state");
        b.expect("server", "guild \\d+ \\S+ Builders", "the backend keeps Builders on disk (/var/village/state)");
        return b.timeLimit(150_000).build();
    }

    // ------------------------------------------------------------------ discord_voice

    static long drained(ScenarioRun r, BlockPos speaker) {
        return r.level().getBlockEntity(r.abs(speaker)) instanceof SpeakerBlockEntity s ? s.audio().samplesDrained() : -1;
    }

    private static final Map<ScenarioRun, long[]> MARK = java.util.Collections.synchronizedMap(new java.util.WeakHashMap<>());

    /** Null once bo's Speaker has stayed under the mark for {@code ms}; fails the run if it plays past it. */
    static String quietFor(ScenarioRun r, long ms) {
        long[] m = MARK.get(r);
        long played = drained(r, BO_SPEAKER);
        if (played > m[0]) {
            r.fail("bo's Speaker played " + (played - m[0]) + " samples it shouldn't have");
            return null;
        }
        return System.currentTimeMillis() - m[1] >= ms ? null : "listening";
    }

    private static void place(ScenarioRun r) {
        ScenarioPlayer p = r.player();
        p.place(VoiceContent.MICROPHONE.get(), r.abs(ALEX_MIC), Direction.NORTH, s -> s.getValue(MicrophoneBlock.FACING) == Direction.NORTH);
        p.place(SpeakerContent.SPEAKER.get(), r.abs(ALEX_SPEAKER), Direction.NORTH, s -> s.getValue(SpeakerBlock.FACING) == Direction.NORTH);
        p.place(VoiceContent.MICROPHONE.get(), r.abs(BO_MIC), Direction.NORTH, s -> s.getValue(MicrophoneBlock.FACING) == Direction.NORTH);
        p.place(SpeakerContent.SPEAKER.get(), r.abs(BO_SPEAKER), Direction.NORTH, s -> s.getValue(SpeakerBlock.FACING) == Direction.NORTH);
        p.place(SpeakerContent.SPEAKER.get(), r.abs(SERVER_SPEAKER), Direction.NORTH, s -> s.getValue(SpeakerBlock.FACING) == Direction.NORTH);
    }

    /** Join General and allow what Discord asks for (the microphone, the speaker). */
    static void joinVoice(Scenario.Builder b, String host) {
        b.await(r -> click(r, host, "Discord", "General"), host + ": click the General voice channel", 15_000);
        // Discord opens the speaker and the microphone: the desktop asks for each (Allow), in either order.
        b.await(r -> {
            var texts = comp(r, host).layerTexts("permission");
            if (texts.stream().anyMatch(t -> t.contains("Allow Discord to use"))) {
                String why = clickOn(r, host, "permission", "Allow");
                return why == null ? "allowed " + texts + "; waiting for the call" : why;
            }
            return shows(r, host, "Voice Connected");
        }, host + ": allow Discord the speaker and the microphone (the desktop asks for each)", 25_000);
        b.await(r -> shows(r, host, "Voice Connected"), host + " is in General (Voice Connected)", 15_000);
    }

    private static Scenario voice() {
        var b = threeComputers("discord_voice",
                "two players in a Discord voice channel: a Speaker beeping beside alex's Microphone is heard on bo's Speaker,"
                        + " 30 blocks away (sent over UDP and forwarded by the backend); not before, and not while alex is muted");
        b.decor(PlayerKit.decor(List.of(ALEX_MIC, ALEX_SPEAKER, BO_MIC, BO_SPEAKER, SERVER_SPEAKER), null, DiscordScenarios::place, null));
        b.note("Also placed for you: a Microphone west and a Speaker east of alex, the same beside bo, and a Speaker east of the"
                + " server, two blocks from alex's Microphone.");
        b.realTime();
        b.send("server", "discordd &");
        b.expect("server", "discordd listening on port 7777", "the backend listens");
        b.send("server", "disco serve &");
        b.expect("server", "disco: answering on UDP port 5354", "disco serve answers");
        repository(b);
        openDiscord(b, "alex", "10.0.8.1");
        b.await(r -> shows(r, "alex", "General"), "alex: the home server, with its General voice channel", 25_000);
        joinVoice(b, "alex");
        openDiscord(b, "bo", "10.0.9.1");
        b.await(r -> shows(r, "bo", "General"), "bo: the home server, with its General voice channel", 25_000);
        joinVoice(b, "bo");

        b.note("Control: the room is quiet; nothing plays on bo's Speaker");
        b.mutate(r -> MARK.put(r, new long[] {drained(r, BO_SPEAKER) + 1600, System.currentTimeMillis()}), "note bo's Speaker");
        b.await(r -> quietFor(r, 2_000), "control: bo's Speaker stays quiet for two seconds", 4_000);
        b.mutate(r -> MARK.put(r, new long[] {drained(r, BO_SPEAKER) - 1600, 0}), "note bo's Speaker again");

        b.note("The server's Speaker beeps beside alex's Microphone; bo hears it on their Speaker");
        b.send("server", "beep 700 5000 --wave sine &");
        b.await(r -> drained(r, BO_SPEAKER) - MARK.get(r)[0] > 16_000 ? null : "bo's Speaker played " + (drained(r, BO_SPEAKER) - MARK.get(r)[0]),
                "bo's Speaker plays what alex's Microphone hears (a second of it)", 15_000);

        b.note("Control: alex mutes; the beep goes on but bo's Speaker goes quiet");
        b.await(r -> click(r, "alex", "Discord", "Mute"), "alex: click Mute", 5_000);
        b.await(r -> shows(r, "alex", "Unmute"), "alex is muted", 5_000);
        b.await(r -> {
            // What was already queued at bo plays out first (at most the jitter buffer and the speaker's queue).
            MARK.put(r, new long[] {drained(r, BO_SPEAKER) + 16_000, System.currentTimeMillis()});
            return null;
        }, "note bo's Speaker (allowing a second already queued)", 1_000);
        b.send("server", "beep 700 3000 --wave sine &");
        b.await(r -> quietFor(r, 3_000), "control: nothing more reaches bo while alex is muted (three seconds of beeping)", 5_000);
        return b.timeLimit(150_000).build();
    }
}
//?}
