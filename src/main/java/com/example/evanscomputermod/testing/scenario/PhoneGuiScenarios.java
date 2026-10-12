package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.ComputerStorage;
import com.example.evanscomputermod.computer.apps.AppInstance;
import com.example.evanscomputermod.computer.apps.BackgroundScheduler;
import com.example.evanscomputermod.computer.ui.Compositor;
import com.example.evanscomputermod.computer.ui.PhoneUi;
import com.example.evanscomputermod.computer.ui.UiManager;
import com.example.evanscomputermod.controller.ControllerData;
import com.example.evanscomputermod.controller.ControllerInput;
import com.example.evanscomputermod.controller.ControllerPackets;
import com.example.evanscomputermod.controller.ControllerServer;
import com.example.evanscomputermod.controller.ControllerState;
import com.example.evanscomputermod.item.ModItems;
import com.example.evanscomputermod.network.ComputerTargets;
import com.example.evanscomputermod.network.MouseInputPacket;
import com.example.evanscomputermod.phone.PhoneHost;
import com.example.evanscomputermod.phone.PhonePackets;
import com.example.evanscomputermod.phone.PhoneServer;
import com.example.evanscomputermod.phone.PhoneState;
import com.example.evanscomputermod.wasm.WasmManager;
import net.minecraft.world.item.ItemStack;

import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.WeakHashMap;

/**
 * The ePhone as a GUI phone (ROADMAP P5 part b, docs/gui/EPHONE.md "Phone
 * shell and lifecycle"), operated by a scenario player through the same calls
 * the game makes: taps and keys through the UUID input path
 * ({@code acceptsInput}: the holder of an awake phone), hotkeys through
 * {@code PhoneServer.deliverHotkey}, the phone moved between hand, hotbar and
 * inventory. The same definitions run as GameTests ({@code ecm_phone}).
 *
 * <ul>
 *   <li>{@code phone_gui}: it boots into the phone shell (no console); Notes
 *       opens from the home grid; text typed in open mode; hotkey slot 1 reaches
 *       the open app (Notes adds the note); Alt+H home suspends it after it
 *       saved its state; the switch-app key cycles open apps without opening
 *       the phone; in the hotbar the open app is suspended and frozen, back in
 *       hand it goes on in the same state.</li>
 *   <li>{@code phone_background}: a refresh wake while the phone sleeps in the
 *       hotbar posts a notification and is cut after its 1 s budget (control:
 *       without {@code background:refresh} nothing runs); a suspended app keeps
 *       playing music on the phone's speaker under the audio exception.</li>
 *   <li>{@code phone_gba}: the GBA app: the documents permission dialog
 *       (answered by tapping Allow), the ROM list, the game running with its
 *       touch buttons, hotkey slot 1 = A; GAMES back to the list, its Settings
 *       page rebinding Start to Space (saved in the app's data folder; control:
 *       A refused the same key), the game started again; a Wireless Controller
 *       paired with the phone and connected to it.</li>
 * </ul>
 */
public final class PhoneGuiScenarios {
    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static final String COUNTER = "dev.ecm.test.counter", REFRESH = "dev.ecm.test.refresh", TONE = "dev.ecm.test.tone";
    static final int HID_H = 0x0B, ALT = 0x4;

    /** Per run. */
    static final class State {
        int count = -1;
        AppInstance app;
        long since;
        int notes;
        BackgroundScheduler.Window window;
        long samples;
        UUID controller;
    }

    private static final Map<ScenarioRun, State> STATES = java.util.Collections.synchronizedMap(new WeakHashMap<>());

    static State st(ScenarioRun r) {
        return STATES.computeIfAbsent(r, k -> new State());
    }

    private PhoneGuiScenarios() {}

    // ------------------------------------------------------------ the phone, as its holder uses it

    static PhoneHost host(ScenarioRun r) {
        return PhoneScenarios.host(r);
    }

    static ComputerInstance computer(ScenarioRun r) {
        PhoneHost h = host(r);
        return h == null ? null : h.instance();
    }

    static UiManager ui(ScenarioRun r) {
        ComputerInstance c = computer(r);
        return c == null ? null : c.getUiManager();
    }

    static Compositor comp(ScenarioRun r) {
        UiManager u = ui(r);
        return u == null ? null : u.compositor();
    }

    static PhoneUi phone(ScenarioRun r) {
        UiManager u = ui(r);
        return u == null ? null : u.phone();
    }

    /** The computer, if the holder may give it input now (UUID input path). */
    static ComputerInstance input(ScenarioRun r) {
        IComputerHost h = ComputerTargets.resolveInput(PhoneScenarios.holder(r), PhoneScenarios.st(r).phone, IComputerHost.DISPLAY_OWN);
        return h == null ? null : ComputerTargets.instance(h);
    }

    /** A tap (mouse as a finger in open mode) at device pixel {@code at}. */
    static void tap(ScenarioRun r, int[] at) {
        ComputerInstance c = input(r);
        if (c == null || at == null) {
            r.fail(c == null ? "the phone refused its holder's touch" : "nothing to tap");
            return;
        }
        c.handlePointerInput(IComputerHost.DISPLAY_OWN, MouseInputPacket.KIND_DOWN, at[0], at[1], (byte) 1, (byte) 0, (byte) 0);
        c.handlePointerInput(IComputerHost.DISPLAY_OWN, MouseInputPacket.KIND_UP, at[0], at[1], (byte) 0, (byte) 0, (byte) 0);
    }

    /** Wait for {@code label} on the shell's {@code layer}, then tap it. */
    static void tapShell(Scenario.Builder b, String layer, String label, String what) {
        b.await(r -> comp(r) != null && comp(r).locateLayerText(layer, label, false) != null ? null
                        : "'" + label + "' not on the " + layer + " layer: " + (comp(r) == null ? "-" : comp(r).layerTexts(layer)),
                "'" + label + "' on the phone's " + layer, 20_000);
        b.mutate(r -> tap(r, comp(r).locateLayerText(layer, label, false)), what);
    }

    /** Alt+key in open mode (the shell's chords). */
    static void altKey(ScenarioRun r, int hid) {
        ComputerInstance c = input(r);
        if (c == null) {
            r.fail("the phone refused its holder's keys");
            return;
        }
        c.handleKeyEvent(hid, ALT, true, false);
        c.handleKeyEvent(hid, ALT, false, false);
    }

    /** A key press and release in open mode. */
    static void key(ScenarioRun r, int hid) {
        ComputerInstance c = input(r);
        if (c == null) {
            r.fail("the phone refused its holder's keys");
            return;
        }
        c.handleKeyEvent(hid, 0, true, false);
        c.handleKeyEvent(hid, 0, false, false);
    }

    static void hotkey(ScenarioRun r, int slot) {
        for (boolean down : new boolean[]{true, false}) {
            if (!PhoneServer.deliverHotkey(PhoneScenarios.holder(r), new PhonePackets.Hotkey(PhoneScenarios.st(r).phone, slot, down))) {
                r.fail("the phone refused its holder's hotkey " + slot);
            }
        }
    }

    /** The app window on screen (between the bars, focused, shown), or null. */
    static Compositor.WindowInfo onScreen(ScenarioRun r, String appId) {
        Compositor c = comp(r);
        if (c == null) return null;
        for (Compositor.WindowInfo w : c.windows()) {
            if (w.appId().equals(appId) && w.focused() && w.visible() && w.mapped() && w.y() == 16) return w;
        }
        return null;
    }

    static AppInstance app(ScenarioRun r, String appId) {
        PhoneUi p = phone(r);
        if (p == null) return null;
        return p.apps().running().stream().filter(i -> i.appId().equals(appId)).findFirst().orElse(null);
    }

    static Path disk(ScenarioRun r) {
        return ComputerStorage.path(r.level().getServer(), PhoneScenarios.st(r).phone);
    }

    static int countOf(ScenarioRun r, String appId) {
        try {
            Path f = disk(r).resolve("var/apps").resolve(appId).resolve("count");
            return Files.exists(f) ? Integer.parseInt(Files.readString(f).trim()) : -1;
        } catch (Exception e) {
            return -1;
        }
    }

    /** Install a probe app (phonetest from the server's programs) as {@code appId}. */
    static void install(ScenarioRun r, String appId, String name, String extra) {
        try {
            Path dir = disk(r).resolve("apps").resolve(appId);
            Files.createDirectories(dir);
            Files.copy(WasmManager.getWasmBinPath().resolve("phonetest.wasm"), dir.resolve("main.wasm"),
                    java.nio.file.StandardCopyOption.REPLACE_EXISTING);
            Files.writeString(dir.resolve("app.toml"), "id = \"" + appId + "\"\nname = \"" + name + "\"\nversion = \"1.0.0\"\n"
                    + "entry = \"main.wasm\"\n" + extra, StandardCharsets.UTF_8);
            phone(r).apps().reindex();
        } catch (Exception e) {
            r.fail("can't install " + appId + ": " + e);
        }
    }

    /** Null once the phone is awake in its shell with the home grid up. */
    static String guiBooted(ScenarioRun r) {
        PhoneHost h = host(r);
        if (h == null || h.state() != PhoneState.AWAKE || !h.isAttached()) return "phone not awake";
        Compositor c = comp(r);
        if (phone(r) == null || c == null || !c.shellConnected()) return "booting to the phone shell";
        if (c.display().logicalW() != 240 || c.display().logicalH() != 400) return "display " + c.display();
        return c.layerTexts("home").contains("Notes") ? null : "home grid " + c.layerTexts("home");
    }

    static Scenario.Builder start(String name, String what) {
        return Scenario.builder(name, what)
                .asPlayer()
                .decor(PhoneScenarios.stage(PhoneScenarios.floor(-2, -2, 2, 2), r -> {
                    PhoneScenarios.givePhone(r);
                    // The battery drains per server tick, and a GameTest server ticks unthrottled
                    // while the phone runs in real time: it would run flat in a long run.
                    com.example.evanscomputermod.phone.PhoneConfig.override("battery.enabled", false);
                }))
                .note("Setup (done for you): a player right-clicks a new ePhone and holds it (batteries off for the run)."
                        + " By hand: hold an ePhone and right-click to open it (every key and the mouse go to the phone).")
                .await(PhoneGuiScenarios::guiBooted, "held: the phone boots into its shell (home grid, status and nav bars, no console)", 40_000);
    }

    static void openApp(Scenario.Builder b, String name, String appId) {
        b.mutate(r -> {
            if (phone(r).foreground() != null) altKey(r, HID_H);
        }, "Alt+H (home) if an app is open");
        tapShell(b, "home", name, "tap " + name + " on the home grid");
        b.await(r -> onScreen(r, appId) != null ? null : "windows " + comp(r).windows(), name + " opens full screen between the bars", 30_000);
    }

    // ------------------------------------------------------------ phone_gui

    private static Scenario gui() {
        var b = start("phone_gui", "the phone boots into its shell; Notes; hotkey slot 1 reaches the open app; the switch-app key;"
                + " the hotbar suspends the open app and it comes back in the same state");
        b.mutate(r -> install(r, COUNTER, "Counter", "hotkeys = [{ slot = 1, action = \"bump\", label = \"Bump\" }]\n"),
                "install a Counter app (counts every 0.25 s of its own run time)");
        openApp(b, "Notes", "dev.ecm.notes");
        b.mutate(r -> {
            Compositor.WindowInfo w = onScreen(r, "dev.ecm.notes");
            tap(r, comp(r).locateText(w.id(), "New note", false));
            ComputerInstance c = input(r);
            if (c != null) c.handleTextEvent("milk");
        }, "tap the note field and type 'milk' (open mode)");
        b.await(r -> comp(r).textsOf(onScreen(r, "dev.ecm.notes").id()).contains("milk") ? null : "texts "
                + comp(r).textsOf(onScreen(r, "dev.ecm.notes").id()), "'milk' in the field", 10_000);
        b.note("In hand: hotkey slot 1 is Notes' 'Add note' (the Controls screen lists it)");
        b.mutate(r -> hotkey(r, 1), "press hotkey slot 1");
        b.await(r -> {
            List<String> t = comp(r).textsOf(onScreen(r, "dev.ecm.notes").id());
            return t.contains("milk") && t.contains("New note") ? null : "texts " + t;
        }, "slot 1 reached Notes: the note is added and the field emptied", 10_000);
        b.mutate(r -> altKey(r, HID_H), "Alt+H: home");
        b.await(r -> {
            AppInstance a = app(r, "dev.ecm.notes");
            Path state = disk(r).resolve("var/apps/dev.ecm.notes/state");
            try {
                return a != null && a.state() == AppInstance.State.SUSPENDED && Files.exists(state) && Files.readString(state).contains("milk")
                        ? null : "notes " + a + ", events " + phone(r).events();
            } catch (Exception e) {
                return e.toString();
            }
        }, "home: Notes saved its state (WillSuspend -> SaveState) and is suspended", 10_000);
        openApp(b, "Counter", COUNTER);
        b.await(r -> countOf(r, COUNTER) >= 3 ? null : "count " + countOf(r, COUNTER), "the counter counts", 20_000);
        b.note("The switch-app key (Controls > ePhone: switch app) cycles open apps without opening the phone");
        b.mutate(r -> hotkey(r, 0), "press the switch-app key");
        b.await(r -> onScreen(r, "dev.ecm.notes") != null ? null : "windows " + comp(r).windows(), "Notes is on screen again", 10_000);
        b.mutate(r -> hotkey(r, 0), "press it again");
        b.await(r -> onScreen(r, COUNTER) != null && app(r, COUNTER).state() == AppInstance.State.RUNNING ? null
                : "windows " + comp(r).windows(), "back to the Counter, running", 10_000);
        b.note("Hotbar: select another slot. The open app is suspended (state saved), the phone sleeps");
        b.mutate(r -> {
            st(r).app = app(r, COUNTER);
            PhoneScenarios.select(PhoneScenarios.holder(r), 1);
        }, "select hotbar slot 2");
        b.await(r -> host(r).state() == PhoneState.ASLEEP && st(r).app.state() == AppInstance.State.SUSPENDED && computer(r).isAsleep()
                ? null : "phone " + host(r).state() + ", app " + st(r).app, "asleep: the Counter is suspended", 10_000);
        b.mutate(r -> st(r).count = countOf(r, COUNTER), "note the count");
        b.waitMs(1_500, "in the hotbar for 1.5 s");
        b.await(r -> {
            if (countOf(r, COUNTER) != st(r).count) r.fail("the counter moved from " + st(r).count + " to " + countOf(r, COUNTER) + " asleep");
            return null;
        }, "frozen while in the hotbar", 1_000);
        b.mutate(r -> PhoneScenarios.select(PhoneScenarios.holder(r), 0), "hold the phone again");
        b.await(r -> {
            int now = countOf(r, COUNTER);
            if (host(r).state() != PhoneState.AWAKE || now <= st(r).count) return "phone " + host(r).state() + ", count " + now;
            if (app(r, COUNTER) != st(r).app) r.fail("the app was restarted");
            if (now > st(r).count + 12) r.fail("the count jumped from " + st(r).count + " to " + now);
            return null;
        }, "awake: the same app goes on from its count", 10_000);
        b.timeLimit(60_000);
        return b.build();
    }

    // ------------------------------------------------------------ phone_background

    private static int notificationsOf(ScenarioRun r, String appId) {
        return (int) comp(r).notifications().stream().filter(n -> n.appId().equals(appId)).count();
    }

    private static Scenario background() {
        var b = start("phone_background", "a refresh wake in the hotbar posts a notification and is cut after its budget;"
                + " music keeps playing in the background under the audio exception");
        b.mutate(r -> {
            install(r, REFRESH, "Refresh", "permissions = [\"notifications\"]\nbackground = { refresh = true }\n");
            install(r, TONE, "Tone", "permissions = [\"peripherals:speaker\"]\nbackground = { modes = [\"audio\"] }\n");
            var perms = phone(r).apps().permissions();
            // Notify needs notifications (allowed when the app first asked; integration's checkNotify).
            perms.setDecision(REFRESH, "notifications", true);
            perms.setDecision(TONE, "peripherals:speaker", true);
            perms.setDecision(TONE, "background:audio", true);
            // Short wakes for the run (a server: every 5 min, 2 s).
            phone(r).apps().background().configure(2_000, 1_000, 10);
        }, "install Refresh (asks for background refresh) and Tone (audio mode; speaker and background audio allowed);"
                + " refresh wakes every 2 s with a 1 s budget for the run");
        openApp(b, "Refresh", REFRESH);
        b.await(r -> countOf(r, REFRESH) >= 2 ? null : "count " + countOf(r, REFRESH), "Refresh runs", 20_000);
        b.mutate(r -> altKey(r, HID_H), "Alt+H: home (Refresh is suspended)");
        b.mutate(r -> PhoneScenarios.select(PhoneScenarios.holder(r), 1), "put the phone in the hotbar (asleep)");
        b.await(r -> host(r).state() == PhoneState.ASLEEP && app(r, REFRESH).state() == AppInstance.State.SUSPENDED ? null
                : "phone " + host(r).state(), "asleep, Refresh suspended", 10_000);
        b.mutate(r -> {
            st(r).count = countOf(r, REFRESH);
            st(r).notes = notificationsOf(r, REFRESH);
        }, "control: background:refresh not allowed yet");
        b.waitMs(2_500, "2.5 s (more than a refresh interval)");
        b.await(r -> {
            if (countOf(r, REFRESH) != st(r).count || notificationsOf(r, REFRESH) != st(r).notes) {
                r.fail("Refresh ran without background:refresh");
            }
            return null;
        }, "nothing ran without the permission", 1_000);
        b.mutate(r -> phone(r).apps().permissions().setDecision(REFRESH, "background:refresh", true),
                "allow Background refresh for Refresh (Settings)");
        b.await(r -> (st(r).window = phone(r).apps().background().currentWindow()) != null ? null : "no window yet",
                "a refresh wake starts while the phone is in the hotbar", 6_000);
        b.await(r -> notificationsOf(r, REFRESH) > st(r).notes && host(r).state() == PhoneState.ASLEEP ? null
                : "notifications " + comp(r).notifications(), "it posts a notification (the phone still in the hotbar)", 6_000);
        b.await(r -> {
            BackgroundScheduler.Window w = st(r).window;
            if (!w.isOver()) return "running " + w.ranMs() + " ms";
            if (!"budget".equals(w.endedBy())) r.fail("the wake ended by " + w.endedBy());
            if (w.ranMs() < 950 || w.ranMs() > 1_600) r.fail("the wake ran " + w.ranMs() + " ms (budget 1000)");
            st(r).count = countOf(r, REFRESH);
            return null;
        }, "the wake is cut after its 1 s budget", 5_000);
        b.waitMs(300, "a moment after the cut");
        b.await(r -> {
            if (countOf(r, REFRESH) != st(r).count && phone(r).apps().background().currentWindow() == null) {
                r.fail("Refresh ran after its budget");
            }
            return null;
        }, "nothing runs after the budget", 1_000);
        b.note("Audio: Tone plays on the phone's own speaker, then goes to the background");
        b.mutate(r -> {
            phone(r).apps().background().configure(3_600_000, 1_000, 10);
            PhoneScenarios.select(PhoneScenarios.holder(r), 0);
        }, "hold the phone (and no more refresh wakes)");
        b.await(r -> host(r).state() == PhoneState.AWAKE ? null : "phone " + host(r).state(), "awake", 5_000);
        openApp(b, "Tone", TONE);
        b.await(r -> host(r).speakerAudio().samplesDrained() > 8_000 ? null : "samples " + host(r).speakerAudio().samplesDrained(),
                "the tone plays (speaker samples)", 20_000);
        b.mutate(r -> altKey(r, HID_H), "Alt+H: home");
        b.await(r -> app(r, TONE).state() == AppInstance.State.SUSPENDED && ui(r).control("phone").contains("mode " + TONE + " audio")
                        && comp(r).layerTexts("status").contains("AUDIO") ? null : "status " + ui(r).control("phone"),
                "Tone suspended with the audio exception on (AUDIO in the status bar)", 10_000);
        b.mutate(r -> PhoneScenarios.select(PhoneScenarios.holder(r), 1), "put the phone in the hotbar");
        b.waitMs(500, "asleep");
        b.mutate(r -> st(r).samples = host(r).speakerAudio().samplesDrained(), "count the samples");
        b.waitMs(3_000, "3 s in the hotbar");
        b.await(r -> {
            long played = host(r).speakerAudio().samplesDrained() - st(r).samples;
            r.say("§f  played " + played + " samples in 3 s (8000/s) while suspended and asleep");
            if (played < 12_000) r.fail("the music stopped in the background: " + played + " samples in 3 s");
            if (host(r).state() != PhoneState.ASLEEP || app(r, TONE).state() != AppInstance.State.SUSPENDED) r.fail("not asleep/suspended");
            return null;
        }, "the music keeps playing (at least half the samples)", 1_000);
        b.timeLimit(60_000);
        return b.build();
    }

    // ------------------------------------------------------------ phone_gba

    private static Scenario gba() {
        var b = start("phone_gba", "the GBA app: permission dialog, ROM list, game with touch buttons, hotkey slot 1 = A, GAMES and key settings, a paired controller");
        b.mutate(r -> {
            try (InputStream in = PhoneGuiScenarios.class.getResourceAsStream("/evanscomputermod/scenarios/gui/arm.gba")) {
                if (in == null) throw new IllegalStateException("arm.gba is missing from the mod jar");
                Path home = disk(r).resolve("home");
                Files.createDirectories(home);
                Files.write(home.resolve("arm.gba"), in.readAllBytes());
            } catch (Exception e) {
                r.fail(e.toString());
            }
            var perms = phone(r).apps().permissions();
            perms.setDecision("dev.ecm.gba", "controller", true);
            perms.setDecision("dev.ecm.gba", "peripherals:speaker", true);
        }, "put a game in the phone's documents (/home/arm.gba); controller and speaker already allowed");
        tapShell(b, "home", "GBA", "tap GBA");
        b.await(r -> String.join(" ", comp(r).layerTexts("overlay")).contains("documents") ? null : "overlay " + comp(r).layerTexts("overlay"),
                "GBA asks to read the documents folder: the permission dialog", 30_000);
        tapShell(b, "overlay", "Allow", "tap Allow");
        b.await(r -> onScreen(r, "dev.ecm.gba") != null && comp(r).textsOf(onScreen(r, "dev.ecm.gba").id()).contains("arm.gba") ? null
                : "windows " + comp(r).windows(), "GBA lists the game", 30_000);
        b.mutate(r -> tap(r, comp(r).locateText(onScreen(r, "dev.ecm.gba").id(), "arm.gba", false)), "tap arm.gba");
        b.await(r -> {
            Compositor.WindowInfo w = onScreen(r, "dev.ecm.gba");
            return w != null && comp(r).textsOf(w.id()).contains("START") && varied(comp(r), w) ? null : "waiting for the game";
        }, "the game runs with its touch buttons (D-pad, A, B, L, R, Start, Select)", 30_000);
        b.note("In hand, the six slots are the game's: 1 A, 2 B, 3 Left, 4 Right, 5 Up, 6 Start");
        b.mutate(r -> hotkey(r, 1), "press hotkey slot 1");
        b.await(r -> phone(r).events().contains("hotkey 1 down -> dev.ecm.gba") ? null : "events " + phone(r).events(),
                "slot 1 reached the game (A)", 5_000);
        b.note("GAMES (between L and R) goes back to the game list; its Settings page sets the keyboard keys");
        b.mutate(r -> tap(r, comp(r).locateText(onScreen(r, "dev.ecm.gba").id(), "GAMES", false)), "tap GAMES");
        b.await(r -> {
            List<String> t = comp(r).textsOf(onScreen(r, "dev.ecm.gba").id());
            return t.contains("arm.gba") && t.contains("Settings") && !t.contains("START") ? null : "texts " + t;
        }, "back at the game list, with Settings", 10_000);
        b.mutate(r -> tap(r, comp(r).locateText(onScreen(r, "dev.ecm.gba").id(), "Settings", false)), "tap Settings");
        b.await(r -> comp(r).textsOf(onScreen(r, "dev.ecm.gba").id()).contains("Controls") ? null : "texts " + comp(r).textsOf(onScreen(r, "dev.ecm.gba").id()),
                "the Controls page lists the keys", 10_000);
        b.mutate(r -> tap(r, comp(r).locateText(onScreen(r, "dev.ecm.gba").id(), "Start", false)), "tap the Start row");
        b.mutate(r -> key(r, 0x2C), "press Space");
        b.await(r -> {
            List<String> t = comp(r).textsOf(onScreen(r, "dev.ecm.gba").id());
            Path keys = disk(r).resolve("var/apps/dev.ecm.gba/keys.json");
            try {
                return t.contains("Space") && !t.contains("Enter") && Files.exists(keys) && Files.readString(keys).contains("\"Start\":44") ? null : "texts " + t;
            } catch (Exception e) {
                return e.toString();
            }
        }, "Start = Space, saved in /var/apps/dev.ecm.gba/keys.json", 10_000);
        b.mutate(r -> tap(r, comp(r).locateText(onScreen(r, "dev.ecm.gba").id(), "A", false)), "tap the A row");
        b.mutate(r -> key(r, 0x2C), "press Space again (control)");
        b.await(r -> comp(r).textsOf(onScreen(r, "dev.ecm.gba").id()).stream().anyMatch(t -> t.contains("already Start")) ? null
                : "texts " + comp(r).textsOf(onScreen(r, "dev.ecm.gba").id()), "control: A can't take Space, the conflict is named", 10_000);
        b.mutate(r -> tap(r, comp(r).locateText(onScreen(r, "dev.ecm.gba").id(), "press a key...", false)), "tap the A row again (cancel)");
        b.await(r -> !comp(r).textsOf(onScreen(r, "dev.ecm.gba").id()).contains("press a key...") ? null : "A still waits",
                "A no longer waits for a key", 10_000);
        b.mutate(r -> tap(r, comp(r).locateText(onScreen(r, "dev.ecm.gba").id(), "Back", false)), "tap Back");
        b.await(r -> comp(r).textsOf(onScreen(r, "dev.ecm.gba").id()).contains("arm.gba") ? null : "texts " + comp(r).textsOf(onScreen(r, "dev.ecm.gba").id()),
                "the game list again", 10_000);
        b.mutate(r -> tap(r, comp(r).locateText(onScreen(r, "dev.ecm.gba").id(), "arm.gba", false)), "tap arm.gba again");
        b.await(r -> comp(r).textsOf(onScreen(r, "dev.ecm.gba").id()).contains("START") ? null : "waiting for the game",
                "the game runs again", 30_000);
        b.note("A Wireless Controller: right-click it with the phone in the other hand to pair");
        b.mutate(r -> {
            var p = PhoneScenarios.holder(r);
            PhoneScenarios.move(r, p, 40);
            ItemStack pad = new ItemStack(ModItems.WIRELESS_CONTROLLER.get());
            r.player().useInAir(pad, false);
            p.getInventory().setItem(0, pad);
            PhoneScenarios.select(p, 0);
            st(r).controller = ControllerData.id(pad);
            if (!PhoneScenarios.st(r).phone.equals(ControllerData.computer(pad))) r.fail("the controller didn't pair with the phone");
            ControllerServer.deliverInput(p, new ControllerPackets.Input(st(r).controller, true, ControllerState.of(Set.of(ControllerInput.A))));
        }, "phone to the off hand, right-click the controller (pairs), press A on it");
        b.await(r -> host(r).controllers().connectedCount() == 1 && host(r).getPeripheralHub().get("controller_1") != null
                && host(r).state() == PhoneState.AWAKE ? null : "controllers " + host(r).controllers().connectedCount(),
                "the controller is connected to the phone (controller_1) and it stays awake in the off hand", 5_000);
        b.timeLimit(60_000);
        return b.build();
    }

    /** True if the game view (top of the window) isn't one flat colour. */
    static boolean varied(Compositor c, Compositor.WindowInfo w) {
        byte[] f = c.frameRgba();
        int first = -1;
        int dw = c.display().deviceW();
        for (int y = w.y() + 6; y < w.y() + 150; y += 3) {
            for (int x = w.x() + 6; x < w.x() + w.w() - 6; x += 3) {
                int i = (y * dw + x) * 4;
                int col = (f[i] & 0xFF) << 16 | (f[i + 1] & 0xFF) << 8 | (f[i + 2] & 0xFF);
                if (first < 0) first = col;
                else if (col != first) return true;
            }
        }
        return false;
    }

    static {
        for (Scenario s : List.of(gui(), background(), gba())) ALL.put(s.name, s);
    }
}
//?}
