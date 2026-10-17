package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.EcmConfig;
import com.example.evanscomputermod.computer.apps.AppInstance;
import com.example.evanscomputermod.computer.ui.Compositor;
import com.example.evanscomputermod.phone.PhoneHost;
import com.example.evanscomputermod.phone.PhoneState;
import com.example.evanscomputermod.speaker.AudioStream;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.Blocks;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.WeakHashMap;

/**
 * The background platform on an ePhone (docs/gui/APPS.md "Background", BACKGROUND_APPS.md),
 * as its holder uses it: probe apps (phonetest) play a tone, talk like a call or wait on a
 * wake socket; the player answers the first-time background prompt on the screen, goes
 * home, pockets the phone, puts it in a chest. The same definitions run as GameTests
 * ({@code ecm_background}).
 *
 * <ul>
 *   <li>{@code background_pocket}: Tone 2's prompt answered "Don't allow": it goes quiet at
 *       home (control); Tone's answered "Allow": it plays on at home, in the hotbar and in the
 *       main inventory (a pocket: asleep, radios on); in a chest the phone is off and the
 *       music stops (control).</li>
 *   <li>{@code background_duck}: Tone plays (media) while Voice holds a call (call mode, a
 *       voice stream talking in 1 s bursts): the music is ducked to 25 % while Voice carries
 *       sound and comes back when it is quiet; a silent stretch of the call doesn't duck it
 *       (control). Then the phone goes into the hotbar: Voice's wake lock keeps the computer
 *       running (not just duty-cycle slices) and the call keeps playing.</li>
 *   <li>{@code background_limit}: with {@code apps.maxBackgroundApps = 2} a third app with a
 *       background mode stops the one whose mode started first, and a notification names it;
 *       with the limit at 3 (an operator's config edit) all three keep their modes
 *       (control).</li>
 *   <li>{@code background_wake}: a Wake app waits on a wake socket (SO_ECM_WAKE) while the
 *       phone is in the pocket; a message from a computer on the Wi-Fi network wakes it and
 *       its notification arrives within seconds, long before the 5-minute refresh; without
 *       background refresh allowed it waits (control).</li>
 * </ul>
 *
 * Exceptions (docs/SCENARIO_EXCEPTIONS.md): the probe apps are copied into the phone's app
 * folder (there is no package for them) and their speaker permission is pre-granted; the
 * wake scenario allows Background refresh in the app manager (the Settings switch); the
 * limit scenario's control sets the server config as an operator edits the file.
 */
public final class BackgroundScenarios {
    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static final String TONE = "dev.ecm.test.tone", TONE2 = "dev.ecm.test2.tone", VOICE = "dev.ecm.test.voice",
            TONE3 = "dev.ecm.test3.tone", WAKE = "dev.ecm.test.wake";
    static final BlockPos CHEST = new BlockPos(1, 1, 1);
    static final int HID_H = 0x0B;

    static final class State {
        long samples;
        long at;
        int notes;
    }

    private static final Map<ScenarioRun, State> STATES = java.util.Collections.synchronizedMap(new WeakHashMap<>());

    static State st(ScenarioRun r) {
        return STATES.computeIfAbsent(r, k -> new State());
    }

    private BackgroundScenarios() {}

    private static PhoneHost host(ScenarioRun r) {
        return PhoneGuiScenarios.host(r);
    }

    private static Compositor comp(ScenarioRun r) {
        return PhoneGuiScenarios.comp(r);
    }

    private static AppInstance app(ScenarioRun r, String id) {
        return PhoneGuiScenarios.app(r, id);
    }

    private static long samples(ScenarioRun r) {
        return host(r).speakerAudio().samplesDrained();
    }

    /** Install a probe app with a speaker it may already use (the permission pre-granted: see the class comment). */
    private static void probe(ScenarioRun r, String id, String name, String extra, boolean speaker) {
        PhoneGuiScenarios.install(r, id, name, extra);
        if (speaker) PhoneGuiScenarios.phone(r).apps().permissions().setDecision(id, "peripherals:speaker", true);
    }

    /** Go home (Alt+H) from the open app. */
    private static void home(Scenario.Builder b) {
        b.mutate(r -> PhoneGuiScenarios.altKey(r, HID_H), "Alt+H: home");
    }

    /** The first-time prompt for {@code name} is on screen: tap {@code answer}. */
    private static void answerPrompt(Scenario.Builder b, String name, String answer) {
        b.await(r -> {
            List<String> t = comp(r).layerTexts("overlay");
            return String.join(" ", t).contains(name) && t.contains(answer) ? null : "overlay " + t;
        }, "the shell asks once: keep " + name + " running in the background?", 15_000);
        PhoneGuiScenarios.tapShell(b, "overlay", answer, "tap " + answer);
    }

    /** At least {@code perSecond / 2} samples a second are drained over {@code ms}: the music plays. */
    private static void plays(Scenario.Builder b, int ms, int perSecond, String what) {
        b.mutate(r -> {
            st(r).samples = samples(r);
            st(r).at = System.currentTimeMillis();
        }, "count the speaker's samples");
        b.waitMs(ms, "listen for " + ms / 1000.0 + " s");
        b.await(r -> {
            long got = samples(r) - st(r).samples;
            long want = (System.currentTimeMillis() - st(r).at) * perSecond / 2000;
            r.say("§f  " + got + " samples in " + (System.currentTimeMillis() - st(r).at) + " ms");
            if (got < want) r.fail("the music stopped: " + got + " samples, wanted " + want);
            return null;
        }, what, 1_000);
    }

    private static void silent(Scenario.Builder b, int ms, String what) {
        b.waitMs(1_000, "a moment for the queue to play out");
        b.mutate(r -> st(r).samples = samples(r), "count the speaker's samples");
        b.waitMs(ms, "listen for " + ms / 1000.0 + " s");
        b.await(r -> {
            long got = samples(r) - st(r).samples;
            if (got > 800) r.fail("still playing: " + got + " samples");
            return null;
        }, what, 1_000);
    }

    // ------------------------------------------------------------ background_pocket

    private static Scenario pocket() {
        var b = PhoneGuiScenarios.start("background_pocket", "the first-time background prompt; music keeps playing at home, in the hotbar"
                + " and in the pocket; Don't allow and a chest stop it");
        b.mutate(r -> {
            probe(r, TONE, "Tone", "permissions = [\"peripherals:speaker\"]\nbackground = { modes = [\"audio\"] }\n", true);
            probe(r, TONE2, "Tone 2", "permissions = [\"peripherals:speaker\"]\nbackground = { modes = [\"audio\"] }\n", true);
        }, "install Tone and Tone 2 (440 Hz tone players that may use the speaker; background audio not decided yet)"
                + " [exception: probe apps copied in, speaker pre-granted; docs/SCENARIO_EXCEPTIONS.md]");
        b.note("Control: Tone 2 plays, the player goes home and answers Don't allow");
        PhoneGuiScenarios.openApp(b, "Tone 2", TONE2);
        b.await(r -> samples(r) > 4_000 ? null : "samples " + samples(r), "Tone 2 plays", 20_000);
        home(b);
        answerPrompt(b, "Tone 2", "Don't allow");
        silent(b, 2_000, "Tone 2 is suspended and silent at home");
        b.await(r -> app(r, TONE2).state() == AppInstance.State.SUSPENDED ? null : "tone 2 " + app(r, TONE2), "Tone 2 suspended", 5_000);
        b.note("Tone plays, the player goes home and answers Allow");
        PhoneGuiScenarios.openApp(b, "Tone", TONE);
        b.await(r -> samples(r) > 4_000 ? null : "samples " + samples(r), "Tone plays", 20_000);
        home(b);
        answerPrompt(b, "Tone", "Allow");
        b.await(r -> app(r, TONE).state() == AppInstance.State.SUSPENDED
                        && Boolean.TRUE.equals(PhoneGuiScenarios.phone(r).apps().permissions().decision(TONE, "background:audio"))
                        && comp(r).layerTexts("status").contains("AUDIO") ? null : "status " + comp(r).layerTexts("status"),
                "the answer is remembered (background:audio) and the status bar shows AUDIO", 10_000);
        plays(b, 2_000, 8_000, "at home Tone keeps playing");
        b.mutate(r -> PhoneScenarios.select(PhoneScenarios.holder(r), 1), "select hotbar slot 2 (the phone in the hotbar)");
        b.await(r -> host(r).state() == PhoneState.ASLEEP ? null : "phone " + host(r).state(), "asleep in the hotbar", 5_000);
        plays(b, 2_000, 8_000, "in the hotbar it keeps playing");
        b.note("Pocket: move the phone into the main inventory");
        b.mutate(r -> PhoneScenarios.move(r, PhoneScenarios.holder(r), 9), "move the phone to an inventory slot");
        b.await(r -> host(r).state() == PhoneState.ASLEEP && host(r).isAttached() ? null : "phone " + host(r).state(),
                "in the pocket: asleep and still attached (its radios on)", 5_000);
        plays(b, 3_000, 8_000, "in the pocket the music keeps playing");
        b.note("Control: put the phone in a chest");
        b.mutate(r -> {
            r.player().place(Blocks.CHEST, r.abs(CHEST), Direction.NORTH);
            r.player().useEmpty(r.abs(CHEST), Direction.UP, false);
            r.player().shiftClick(27);
            r.player().closeScreen();
        }, "place a chest, open it, shift-click the phone in and close it");
        b.await(r -> host(r).state() == PhoneState.OFF ? null : "phone " + host(r).state(), "in the chest the phone is off", 5_000);
        silent(b, 2_000, "the music stops");
        b.timeLimit(90_000);
        return b.build();
    }

    // ------------------------------------------------------------ background_duck

    /** Tone's stream on the phone's speaker, or null. */
    private static AudioStream toneStream(ScenarioRun r) {
        AppInstance t = app(r, TONE);
        if (t == null) return null;
        for (AudioStream s : host(r).speakerAudio().streams()) {
            AppInstance a = PhoneGuiScenarios.phone(r).apps().instanceOf(s.pid());
            if (a == t && !s.isClosed()) return s;
        }
        return null;
    }

    private static Scenario duck() {
        var b = PhoneGuiScenarios.start("background_duck", "music on the phone is ducked while a call carries sound and comes back when it is quiet");
        b.mutate(r -> {
            probe(r, TONE, "Tone", "permissions = [\"peripherals:speaker\"]\nbackground = { modes = [\"audio\"] }\n", true);
            probe(r, VOICE, "Voice", "permissions = [\"peripherals:speaker\"]\nbackground = { modes = [\"call\"] }\n", true);
        }, "install Tone (music) and Voice (a call that talks 1 s, then is quiet 1 s)"
                + " [exception: probe apps copied in, speaker pre-granted; docs/SCENARIO_EXCEPTIONS.md]");
        PhoneGuiScenarios.openApp(b, "Tone", TONE);
        b.await(r -> samples(r) > 4_000 ? null : "samples " + samples(r), "Tone plays", 20_000);
        home(b);
        answerPrompt(b, "Tone", "Allow");
        PhoneGuiScenarios.openApp(b, "Voice", VOICE);
        b.await(r -> PhoneGuiScenarios.ui(r).control("phone").contains("mode " + VOICE + " call") ? null : PhoneGuiScenarios.ui(r).control("phone"),
                "Voice holds a call (call mode)", 15_000);
        b.await(r -> {
            AudioStream t = toneStream(r);
            return t != null && t.ducked() ? null : "tone " + t;
        }, "while Voice talks, Tone's stream is ducked (to 25 %)", 10_000);
        b.await(r -> {
            AudioStream t = toneStream(r);
            return t != null && !t.ducked() ? null : "tone " + t;
        }, "control: in the call's quiet second Tone comes back to full volume", 10_000);
        b.await(r -> {
            AudioStream t = toneStream(r);
            return t != null && t.ducked() ? null : "tone " + t;
        }, "Voice talks again: ducked again", 10_000);
        b.mutate(r -> r.say("§f  mixer: " + PhoneGuiScenarios.computer(r).volumeMixer().text().replace('\n', ';')), "read the Volume Mixer");
        b.note("The call in the hotbar: the player goes home, allows Voice in the background and puts the phone away");
        home(b);
        answerPrompt(b, "Voice", "Allow");
        b.mutate(r -> PhoneScenarios.select(PhoneScenarios.holder(r), 1), "select hotbar slot 2 (the phone in the hotbar)");
        b.await(r -> host(r).state() == PhoneState.ASLEEP ? null : "phone " + host(r).state(), "the phone is asleep in the hotbar", 5_000);
        b.await(r -> {
            String locks = PhoneGuiScenarios.ui(r).control("wakelock");
            return locks.contains(VOICE) ? null : locks;
        }, "Voice holds a wake lock for its call (/dev/uictl wakelock lists it)", 10_000);
        b.mutate(r -> st(r).at = System.currentTimeMillis(), "watch the computer");
        b.await(r -> {
            if (PhoneGuiScenarios.computer(r).isAsleep()) r.fail("the computer slept during the call");
            return System.currentTimeMillis() - st(r).at >= 3_000 ? null : "watching";
        }, "the computer keeps running for 3 s (asleep, it would only get short slices)", 6_000);
        plays(b, 2_000, 8_000, "and the call keeps playing");
        b.timeLimit(120_000);
        return b.build();
    }

    // ------------------------------------------------------------ background_limit

    private static String modes(ScenarioRun r) {
        return PhoneGuiScenarios.phone(r).apps().background().activeModes().toString();
    }

    private static Scenario limit() {
        var b = PhoneGuiScenarios.start("background_limit", "with maxBackgroundApps = 2 a third background app stops the oldest and a"
                + " notification names it; with 3 all keep running");
        b.mutate(r -> {
            EcmConfig.setAppsMaxBackgroundApps(2);
            String extra = "permissions = [\"peripherals:speaker\"]\nbackground = { modes = [\"audio\"] }\n";
            probe(r, TONE, "Tone", extra, true);
            probe(r, TONE2, "Tone 2", extra, true);
            probe(r, TONE3, "Tone 3", extra, true);
        }, "server config apps.maxBackgroundApps = 2 (the default); install Tone, Tone 2 and Tone 3"
                + " [exception: probe apps copied in, speaker pre-granted; docs/SCENARIO_EXCEPTIONS.md]");
        for (String[] t : new String[][]{{"Tone", TONE}, {"Tone 2", TONE2}}) {
            PhoneGuiScenarios.openApp(b, t[0], t[1]);
            b.await(r -> app(r, t[1]) != null && app(r, t[1]).activeModes().contains("audio") ? null : "modes " + modes(r), t[0] + " plays (audio mode)", 20_000);
            home(b);
            answerPrompt(b, t[0], "Allow");
        }
        b.mutate(r -> st(r).notes = comp(r).notifications().size(), "two apps play in the background");
        PhoneGuiScenarios.openApp(b, "Tone 3", TONE3);
        b.await(r -> {
            boolean stopped = !app(r, TONE).activeModes().contains("audio");
            boolean named = comp(r).notifications().stream().anyMatch(n -> n.appId().equals(TONE) && n.body().contains("Tone 3"));
            return stopped && named && app(r, TONE2).activeModes().contains("audio") ? null : "modes " + modes(r) + ", notes " + comp(r).notifications();
        }, "Tone 3 starts its mode: Tone (whose mode started first) is stopped and 'Tone was paused to let Tone 3 run' is posted", 20_000);
        b.note("Control: an operator raises apps.maxBackgroundApps to 3 in serverconfig/evanscomputermod-server.toml");
        b.mutate(r -> EcmConfig.setAppsMaxBackgroundApps(3), "set apps.maxBackgroundApps = 3 [exception: the config set as an operator edits the file]");
        PhoneGuiScenarios.openApp(b, "Tone", TONE);
        b.await(r -> app(r, TONE).activeModes().contains("audio") ? null : "modes " + modes(r), "Tone plays again", 20_000);
        home(b);
        b.waitMs(1_500, "all three at home");
        b.await(r -> {
            for (String id : List.of(TONE, TONE2, TONE3)) {
                if (!app(r, id).activeModes().contains("audio")) return "modes " + modes(r);
            }
            return null;
        }, "with the limit at 3 all three keep their audio mode", 5_000);
        b.mutate(r -> EcmConfig.setAppsMaxBackgroundApps(2), "back to the default (2)");
        b.timeLimit(120_000);
        return b.build();
    }

    // ------------------------------------------------------------ background_wake

    static final String SSID = "ecm-wake", GW_IP = "192.168.79.1", PHONE_IP = "192.168.79.2";
    static final BlockPos AP = new BlockPos(3, 1, 1), GW = new BlockPos(3, 1, 4);
    static final List<BlockPos> CABLE = List.of(new BlockPos(3, 0, 4), new BlockPos(3, 0, 3), new BlockPos(3, 0, 2), new BlockPos(3, 0, 1));

    private static int wakeNotes(ScenarioRun r) {
        return (int) comp(r).notifications().stream().filter(n -> n.appId().equals(WAKE) && n.title().startsWith("Wake:")).count();
    }

    /** Walk to gw with an empty hand: select an empty hotbar slot first (the phone stays in the hotbar). */
    private static void toGw(Scenario.Builder b) {
        b.mutate(r -> PhoneScenarios.select(PhoneScenarios.holder(r), 2), "select an empty hotbar slot (the phone stays in slot 1) and walk to gw");
    }

    /** Close gw's screen and hold the phone again (typing on gw opened its screen). */
    private static void backToPhone(Scenario.Builder b) {
        b.mutate(r -> {
            r.player().closeScreen();
            PhoneScenarios.select(PhoneScenarios.holder(r), PhoneScenarios.slotOf(PhoneScenarios.holder(r), PhoneScenarios.st(r).phone));
        }, "close gw's screen (Escape) and look at the phone in hand");
        b.await(r -> host(r).state() == PhoneState.AWAKE ? null : "phone " + host(r).state(), "the phone is awake in hand", 5_000);
    }

    /** gw's Python sends one line over the Wake app's connection. */
    private static void sendLine(Scenario.Builder b, String text) {
        PlayerKit.py(b, "gw", "net.tcp_send(c, b'" + text + "\\n')", "gw sends '" + text + "' to the phone");
    }

    private static Scenario wake() {
        List<BlockPos> floor = new ArrayList<>(PlayerKit.box(-2, 0, -2, 6, 0, 6));
        floor.removeAll(CABLE);
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(CABLE);
        foot.add(AP);
        var b = Scenario.builder("background_wake", "a message on a wake socket wakes a suspended app in the pocket at once")
                .asPlayer()
                .realTime()
                .host("gw", GW, GW_IP + "/24")
                .decor(PlayerKit.decor(foot, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()), r -> {
                    for (BlockPos p : CABLE) r.player().place(com.example.evanscomputermod.block.ModBlocks.NETWORK_CABLE.get(), r.abs(p), Direction.UP);
                    PlayerKit.accessPoint(r, AP, Direction.WEST, SSID, null, 6, 0);
                }, PhoneScenarios::cleanup))
                .note("Setup (done for you): gw computer cabled to an open Access Point (" + SSID + "); a player holds a new phone")
                .send("gw", "ifconfig eth0 " + GW_IP + "/24")
                .expect("gw", "eth0: inet " + GW_IP.replace(".", "\\.") + "/24", "gw has " + GW_IP)
                .mutate(r -> {
                    PhoneScenarios.givePhone(r);
                    com.example.evanscomputermod.phone.PhoneConfig.override("battery.enabled", false);
                }, "the player takes a new phone and holds it (batteries off for the run)")
                .await(PhoneGuiScenarios::guiBooted, "the phone boots into its shell", 40_000);
        PhoneScenarios.openTerminal(b);
        b.mutate(r -> PhoneScenarios.type(r, PhoneScenarios.holder(r), "wifi connect " + SSID + "\n"), "in the phone's Terminal: wifi connect " + SSID);
        // wifi says it joined, then (no DHCP here) how to set an address by hand; the shell prompt follows.
        b.await(r -> {
            String s = PhoneScenarios.screen(r);
            return s.stripTrailing().endsWith("/ >") && (s.contains(SSID) || s.contains("address by hand")) ? null : s;
        }, "the phone joins " + SSID + " (no DHCP on this network: it says to set an address by hand)", 40_000);
        PhoneScenarios.phoneRun(b, "ifconfig wlan0 " + PHONE_IP + "/24", "wlan0: inet " + PHONE_IP.replace(".", "\\."), null, "the phone has " + PHONE_IP, 10_000);
        PhoneScenarios.phoneRun(b, "ping " + GW_IP + " -n 2", "packets sent, [12] received", null, "the phone pings gw through the Access Point", 20_000);
        b.note("gw: a tiny message server in Python, waiting for the Wake app to connect (like a chat backend)");
        toGw(b);
        b.send("gw", "python");
        b.expect("gw", PlayerKit.PY_PROMPT, "gw: Python REPL");
        PlayerKit.py(b, "gw", "import net", "gw: import net");
        PlayerKit.py(b, "gw", "s = net.tcp_listen(7999)", "gw listens on 7999");
        b.send("gw", "c = net.tcp_accept(s); print('accepted')");
        backToPhone(b);
        b.mutate(r -> PhoneGuiScenarios.install(r, WAKE, "Wake", "permissions = [\"network\", \"notifications\"]\nbackground = { refresh = true }\n"),
                "install Wake (keeps a connection to gw:7999 as a wake socket) [exception: probe app copied in; docs/SCENARIO_EXCEPTIONS.md]");
        PhoneGuiScenarios.openApp(b, "Wake", WAKE);
        b.await(r -> String.join(" ", comp(r).layerTexts("overlay")).contains("network") ? null : "overlay " + comp(r).layerTexts("overlay"),
                "Wake asks for the network", 20_000);
        PhoneGuiScenarios.tapShell(b, "overlay", "Allow", "tap Allow");
        b.expect("gw", "^accepted", "gw: the Wake app connected");
        toGw(b);
        sendLine(b, "hello");
        backToPhone(b);
        b.await(r -> {
            List<String> t = comp(r).layerTexts("overlay");
            if (t.contains("Allow")) PhoneGuiScenarios.tap(r, comp(r).locateLayerText("overlay", "Allow", false));
            return wakeNotes(r) >= 1 ? null : "overlay " + t;
        }, "back on the phone, the open Wake app posts 'Wake: hello' (tapping Allow for notifications)", 20_000);
        home(b);
        b.mutate(r -> PhoneScenarios.move(r, PhoneScenarios.holder(r), 9), "pocket the phone (main inventory)");
        b.await(r -> host(r).state() == PhoneState.ASLEEP && app(r, WAKE).state() == AppInstance.State.SUSPENDED ? null
                : "phone " + host(r).state() + ", wake " + app(r, WAKE), "in the pocket: asleep, Wake suspended", 10_000);
        b.note("Control: Background refresh is not allowed for Wake yet: a message waits");
        b.mutate(r -> st(r).notes = wakeNotes(r), "count Wake's notifications");
        sendLine(b, "early");
        b.waitMs(4_000, "4 s");
        b.await(r -> {
            if (wakeNotes(r) != st(r).notes) r.fail("Wake ran without Background refresh");
            return null;
        }, "no notification without Background refresh", 1_000);
        b.mutate(r -> PhoneGuiScenarios.phone(r).apps().permissions().setDecision(WAKE, "background:refresh", true),
                "allow Background refresh for Wake (Settings > Apps) [exception: set in the app manager, docs/SCENARIO_EXCEPTIONS.md]");
        b.mutate(r -> st(r).at = System.currentTimeMillis(), "start the clock");
        sendLine(b, "now");
        b.await(r -> {
            boolean now = comp(r).notifications().stream().anyMatch(n -> n.appId().equals(WAKE) && n.title().equals("Wake: now"));
            if (!now) return "notes " + comp(r).notifications();
            long ms = System.currentTimeMillis() - st(r).at;
            r.say("§f  the notification arrived " + ms + " ms after the message (the refresh interval is 5 min)");
            return host(r).state() == PhoneState.ASLEEP ? null : "phone " + host(r).state();
        }, "the wake socket wakes Wake at once: 'Wake: now' is posted with the phone still in the pocket", 10_000);
        b.timeLimit(120_000);
        return b.build();
    }

    static {
        for (Scenario s : List.of(pocket(), duck(), limit(), wake())) ALL.put(s.name, s);
    }
}
//?}
