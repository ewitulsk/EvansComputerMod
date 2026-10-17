package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.apps.AppInstance;
import com.example.evanscomputermod.computer.ui.Compositor;
import com.example.evanscomputermod.speaker.SpeakerBlock;
import com.example.evanscomputermod.speaker.SpeakerBlockEntity;
import com.example.evanscomputermod.speaker.SpeakerContent;
import com.example.evanscomputermod.wasm.WasmManager;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.WeakHashMap;

/**
 * The desktop's tray (docs/gui/APPS.md "On a Terminal"), as a player uses it; also the
 * {@code ecm_apps} GameTest {@code desktop_tray}.
 *
 * <p>{@code desktop_tray}: a Terminal with a Speaker runs Voice, a probe app holding a call
 * (the {@code call} background mode, a voice stream on the Speaker). The player closes its
 * window with the title bar's x: the window goes, but Voice keeps running and its call keeps
 * playing, with an icon in the taskbar's tray. A click on the icon brings the window back;
 * closing it again and choosing Quit from the icon's right-click menu ends the app and the call
 * (the Speaker goes quiet, the icon goes away). Control: an app without a background mode
 * (Notes) closes for real.
 */
public final class DesktopTrayScenarios {
    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static final BlockPos PC = new BlockPos(0, 1, 0), SPEAKER = new BlockPos(-1, 1, 0);
    static final String VOICE = "dev.ecm.test.voice";

    static final class State {
        List<Integer> before = new ArrayList<>();
        int[] icon;
        long samples;
    }

    private static final Map<ScenarioRun, State> STATES = java.util.Collections.synchronizedMap(new WeakHashMap<>());

    private static State st(ScenarioRun r) {
        return STATES.computeIfAbsent(r, k -> new State());
    }

    private DesktopTrayScenarios() {}

    private static ComputerInstance pc(ScenarioRun r) {
        return AppScenarios.computer(r, "pc");
    }

    private static Compositor comp(ScenarioRun r) {
        return DiscordScenarios.comp(r, "pc");
    }

    private static long samples(ScenarioRun r) {
        return r.level().getBlockEntity(r.abs(SPEAKER)) instanceof SpeakerBlockEntity s ? s.audio().samplesDrained() : -1;
    }

    private static AppInstance voice(ScenarioRun r) {
        return pc(r).getAppManager().running().stream().filter(i -> i.appId().equals(VOICE)).findFirst().orElse(null);
    }

    private static Compositor.WindowInfo window(ScenarioRun r, String appId) {
        for (Compositor.WindowInfo w : comp(r).windows()) if (w.appId().equals(appId) && w.mapped()) return w;
        return null;
    }

    /** Click the x on the title bar of {@code appId}'s window (the desktop draws frames as its own windows). */
    private static String closeWindow(ScenarioRun r, String appId) {
        Compositor c = comp(r);
        Compositor.WindowInfo app = window(r, appId);
        if (app == null) return "no " + appId + " window";
        for (Compositor.WindowInfo f : c.windows()) {
            if (!f.title().startsWith("frame:") || !f.visible() || !c.textsOf(f.id()).contains(app.title())) continue;
            int[] at = c.locateText(f.id(), "x", false);
            if (at == null) return "no close button on " + f.title();
            r.click("pc", at[0], at[1]);
            return null;
        }
        return "no frame for " + app.title();
    }

    private static List<Integer> hitIds(ScenarioRun r) {
        List<Integer> ids = new ArrayList<>();
        for (int[] h : comp(r).layerHits("taskbar")) ids.add(h[0]);
        return ids;
    }

    private static Scenario tray() {
        var b = Scenario.builder("desktop_tray", "closing an app holding a call hides it to the tray; its icon brings it back; Quit ends the call")
                .host("pc", PC, "-")
                .asPlayer()
                .realTime()
                .decor(PlayerKit.decor(List.of(SPEAKER), null, r -> r.player().place(SpeakerContent.SPEAKER.get(), r.abs(SPEAKER),
                        Direction.SOUTH, s -> s.getValue(SpeakerBlock.FACING) == Direction.SOUTH), null))
                .note("Setup (done for you): a Speaker placed west of the computer");
        b.mutate(r -> {
            try {
                Path dir = pc(r).storagePath().resolve("apps").resolve(VOICE);
                Files.createDirectories(dir);
                Files.copy(WasmManager.getWasmBinPath().resolve("phonetest.wasm"), dir.resolve("main.wasm"),
                        java.nio.file.StandardCopyOption.REPLACE_EXISTING);
                Files.writeString(dir.resolve("app.toml"), "id = \"" + VOICE + "\"\nname = \"Voice\"\nversion = \"1.0.0\"\nentry = \"main.wasm\"\n"
                        + "permissions = [\"peripherals:speaker\"]\nbackground = { modes = [\"call\"] }\n", StandardCharsets.UTF_8);
                pc(r).getAppManager().permissions().setDecision(VOICE, "peripherals:speaker", true);
                pc(r).getAppManager().reindex();
            } catch (Exception e) {
                r.fail("can't install Voice: " + e);
            }
        }, "install Voice, a probe app that holds a call [exception: probe app copied in, speaker pre-granted; docs/SCENARIO_EXCEPTIONS.md]");
        b.send("pc", "gui");
        b.expect("pc", "^gui: started", "the desktop starts", "(?i)unknown command|no GUI device");
        b.await(r -> comp(r) != null && comp(r).shellConnected() ? null : "waiting for the shell", "the desktop is up", 15_000);
        b.waitMs(2_500, "the taskbar settles (its speaker icon appears)");
        b.mutate(r -> st(r).before = hitIds(r), "note the taskbar's icons");
        b.await(r -> DiscordScenarios.clickOn(r, "pc", "taskbar", "Apps"), "click Apps on the taskbar", 10_000);
        b.await(r -> DiscordScenarios.clickOn(r, "pc", "popup", "Voice"), "click Voice in the launcher", 5_000);
        b.await(r -> window(r, VOICE) != null && voice(r) != null && voice(r).activeModes().contains("call") ? null
                : "voice " + voice(r), "Voice opens and holds a call (call mode)", 20_000);
        b.await(r -> samples(r) > 8_000 ? null : "samples " + samples(r), "the call plays on the Speaker", 15_000);
        b.note("Close Voice's window with the x on its title bar");
        b.await(r -> closeWindow(r, VOICE), "click x", 5_000);
        b.await(r -> {
            Compositor.WindowInfo w = comp(r).windows().stream().filter(x -> x.appId().equals(VOICE)).findFirst().orElse(null);
            if (w == null || w.visible() || voice(r) == null) return "voice window " + w + ", app " + voice(r);
            List<int[]> now = new ArrayList<>(comp(r).layerHits("taskbar"));
            now.removeIf(h -> st(r).before.contains(h[0]));
            if (now.isEmpty()) return "no new icon in the taskbar";
            st(r).icon = now.get(0);
            st(r).samples = samples(r);
            return null;
        }, "the window is hidden, not closed: Voice runs on with an icon in the tray", 10_000);
        b.waitMs(1_500, "listen");
        b.await(r -> samples(r) > st(r).samples + 8_000 ? null : "samples " + samples(r), "the call keeps playing", 5_000);
        b.note("Click the tray icon: Voice's window comes back");
        b.mutate(r -> r.click("pc", st(r).icon[1], st(r).icon[2]), "click the tray icon");
        b.await(r -> comp(r).windows().stream().anyMatch(w -> w.appId().equals(VOICE) && w.visible()) ? null : "windows " + comp(r).windows(),
                "Voice's window is shown again", 10_000);
        b.note("Close it again, then Quit from the icon's right-click menu");
        b.await(r -> closeWindow(r, VOICE), "click x", 5_000);
        b.await(r -> comp(r).windows().stream().noneMatch(w -> w.appId().equals(VOICE) && w.visible()) ? null : "still shown", "hidden again", 5_000);
        b.mutate(r -> r.rightClick("pc", st(r).icon[1], st(r).icon[2]), "right-click the tray icon");
        b.await(r -> DiscordScenarios.clickOn(r, "pc", "popup", "Quit"), "click Quit", 5_000);
        b.await(r -> voice(r) == null ? null : "voice " + voice(r), "Voice quits: the call ends", 10_000);
        b.waitMs(1_000, "what was queued (half a second) plays out");
        b.mutate(r -> st(r).samples = samples(r), "count the Speaker's samples");
        b.waitMs(1_500, "listen");
        b.await(r -> samples(r) <= st(r).samples + 400 ? null : "still playing: " + (samples(r) - st(r).samples), "the Speaker is quiet", 3_000);
        b.await(r -> hitIds(r).stream().noneMatch(id -> !st(r).before.contains(id)) ? null : "taskbar " + comp(r).layerTexts("taskbar"),
                "the tray icon is gone", 5_000);
        b.note("Control: Notes has no background mode, so its x closes it for real");
        b.await(r -> DiscordScenarios.clickOn(r, "pc", "taskbar", "Apps"), "click Apps", 10_000);
        b.await(r -> DiscordScenarios.clickOn(r, "pc", "popup", "Notes"), "click Notes", 5_000);
        b.await(r -> window(r, "notes") != null ? null : "windows " + comp(r).windows(), "Notes opens", 15_000);
        b.await(r -> closeWindow(r, "notes"), "click x on Notes", 5_000);
        b.await(r -> comp(r).windows().stream().noneMatch(w -> w.appId().equals("notes")) ? null : "windows " + comp(r).windows(),
                "Notes closes (no tray icon)", 10_000);
        return b.timeLimit(90_000).build();
    }

    static {
        for (Scenario s : List.of(tray())) ALL.put(s.name, s);
    }
}
//?}
