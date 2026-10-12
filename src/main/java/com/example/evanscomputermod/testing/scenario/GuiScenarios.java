package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.ui.Compositor;
import com.example.evanscomputermod.computer.ui.UiConnection;
import com.example.evanscomputermod.computer.ui.UiManager;
import com.example.evanscomputermod.computer.ui.proto.UiProto;
import com.example.evanscomputermod.radio.sdr.RadioSdrContent;
import com.example.evanscomputermod.radio.sdr.SdrBlock;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.Blocks;

import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.WeakHashMap;

/**
 * GUI scenarios (1.21.1), spawnable with {@code /ecm scenario spawn <name>}
 * and run as the {@code ecm_gui} GameTests.
 *
 * <p>{@code gui_basics}: a Terminal with a Standard SDR on its east side. Its
 * {@code /etc/ui.toml} tells the test shell to open Notes, {@code gba} (a test
 * ROM) and {@code waterfall} (on the SDR). Typing {@code gui} starts the
 * compositor; the test shell tiles the three windows under its bar. Then:
 * killing the shell (safe mode, the shell back within a second),
 * Ctrl+Alt+Backspace to the console and back (both keep running), and a
 * control: a client sending garbage loses only its own connection.
 * <pre>
 *   y=0 floor x -3..3, z -2..2; Terminal (0,1,0) facing north; SDR (1,1,0) facing south.
 * </pre>
 */
public final class GuiScenarios {
    private static final BlockPos PC = new BlockPos(0, 1, 0);
    private static final BlockPos SDR = PC.east();
    static final String UI_TOML = String.join("\n",
            "# The test shell opens these at start (scenario gui_basics).",
            "autostart = false",
            "shell = \"test-shell\"",
            "shell_args = [\"--launch\", \"notes\", \"--launch\", \"gba:arm.gba\", \"--launch\", \"waterfall:100M:--size:200x120:--rate:48000\"]",
            "theme = \"ecm-default\"",
            "[screen]",
            "mode = \"mirror\"",
            "");
    /** gui_client_render: the test shell opens Notes only (gba is launched later by the scenario). */
    static final String UI_TOML_NOTES = String.join("\n",
            "# The test shell opens Notes (scenario gui_client_render).",
            "autostart = false",
            "shell = \"test-shell\"",
            "shell_args = [\"--launch\", \"notes\"]",
            "theme = \"ecm-default\"",
            "");

    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();
    /** When each run killed its shell (for the "back within a second" check). */
    private static final Map<ScenarioRun, Long> KILLED_AT = new WeakHashMap<>();
    private static final Map<ScenarioRun, UiConnection> BROKEN = new WeakHashMap<>();
    /** gui_client_render: the player whose client draws the GUI, and the typing run. */
    private static final Map<ScenarioRun, com.example.evanscomputermod.testing.SceneViewerMirror> VIEWER = new WeakHashMap<>();
    private static final Map<ScenarioRun, long[]> TYPING = new WeakHashMap<>();
    /** Keystrokes and their rate in the typing check (about 100 words a minute). */
    static final int KEYS = 24, KEYS_PER_SECOND = 8;

    static {
        Scenario s = guiBasics();
        ALL.put(s.name, s);
        Scenario c = guiClientRender();
        ALL.put(c.name, c);
    }

    private GuiScenarios() {}

    static ComputerInstance computer(ScenarioRun r) {
        TerminalBlockEntity be = r.terminal("pc");
        return be == null ? null : be.getComputer();
    }

    static UiManager ui(ScenarioRun r) {
        ComputerInstance c = computer(r);
        return c == null ? null : c.getUiManager();
    }

    static Compositor comp(ScenarioRun r) {
        UiManager u = ui(r);
        return u == null ? null : u.compositor();
    }

    static List<Compositor.WindowInfo> mapped(ScenarioRun r) {
        Compositor c = comp(r);
        if (c == null) return List.of();
        return c.windows().stream().filter(w -> w.mapped() && w.placed()).toList();
    }

    /** Null when every window is drawn, side by side; otherwise what's wrong. */
    static String tiled(ScenarioRun r, int n) {
        List<Compositor.WindowInfo> m = new ArrayList<>(mapped(r));
        if (m.size() != n) return m.size() + " of " + n + " windows drawn";
        m.sort(java.util.Comparator.comparingInt(Compositor.WindowInfo::x));
        for (int i = 0; i + 1 < m.size(); i++) {
            if (m.get(i).x() + m.get(i).w() > m.get(i + 1).x()) return "windows overlap: " + m;
        }
        return null;
    }

    /** Null if the Terminal display shows varied pixels inside window {@code app}'s rectangle. */
    static String drawing(ScenarioRun r, String app) {
        Compositor.WindowInfo w = mapped(r).stream().filter(x -> x.appId().equals(app)).findFirst().orElse(null);
        if (w == null) return app + " has no window";
        TerminalDisplay d = r.terminal("pc").getDisplay();
        byte[] px;
        synchronized (d) {
            if (d.getDisplayMode() != 1 || d.getGfxWidth() != 640 || d.getPixelFormat() != TerminalDisplay.PIXEL_FORMAT_RGBA8888) {
                return "the Terminal doesn't show the GUI";
            }
            px = d.getPixelData().clone();
        }
        int first = -1;
        for (int y = w.y() + 2; y < w.y() + w.h() - 2; y += 2) {
            for (int x = w.x() + 2; x < w.x() + w.w() - 2; x += 2) {
                int i = (y * 640 + x) * 4;
                int c = (px[i] & 0xFF) << 16 | (px[i + 1] & 0xFF) << 8 | (px[i + 2] & 0xFF);
                if (first < 0) first = c;
                else if (c != first) return null;
            }
        }
        return app + "'s window is one flat colour";
    }

    private static Scenario guiBasics() {
        return Scenario.builder("gui_basics",
                        "the GUI on a Terminal: the test shell tiles Notes, gba and waterfall; safe mode; the console switch")
                .host("pc", PC, "-")
                .decor(new Desk(UI_TOML))
                .realTime()
                .note("Control: without the GUI there is no compositor, so a GUI app refuses to run")
                .send("pc", "notes")
                .expect("pc", "no GUI is running", "notes says there is no GUI")
                .note("Start the GUI: /etc/ui.toml makes the test shell open Notes, gba and waterfall")
                .send("pc", "gui")
                .expect("pc", "^gui: started", "the compositor and the test shell started")
                .await(r -> comp(r) != null && comp(r).shellConnected() ? null : "waiting for the shell", "the test shell connected", 15_000)
                .await(r -> tiled(r, 3), "three windows tiled side by side under the shell's bar", 30_000)
                .await(r -> drawing(r, "gba"), "gba draws the test ROM in its window (legacy gfx, unchanged)", 30_000)
                .await(r -> drawing(r, "waterfall"), "waterfall draws the SDR spectrum in its window", 30_000)
                .mutate(r -> EvansComputerMod.LOGGER.info("[gui_basics] {}", ui(r).status().strip()), "log the windows")
                .note("Kill the shell: safe mode keeps the focused window usable, the shell restarts")
                .mutate(r -> {
                    KILLED_AT.put(r, System.currentTimeMillis());
                    computer(r).getProcessManager().kill(comp(r).shellPid());
                }, "kill the test shell's process")
                .await(r -> comp(r).safeMode() ? null : "not in safe mode", "safe mode (focused window maximized, plain bar)", 2_000)
                .await(r -> comp(r).shellConnected() && !comp(r).safeMode() ? null : "shell still away", "the shell back", 3_000)
                .await(r -> {
                    long ms = System.currentTimeMillis() - KILLED_AT.getOrDefault(r, 0L);
                    return ms <= 1_500 ? null : "the shell took " + ms + " ms to come back";
                }, "the shell came back within a second (plus a tick of slack)", 100)
                .await(r -> tiled(r, 3), "the new shell tiled the windows again", 10_000)
                .note("Ctrl+Alt+Backspace: the console comes back; the GUI keeps running")
                .mutate(r -> chord(r), "press Ctrl+Alt+Backspace")
                .await(r -> !ui(r).guiVisible() && r.terminal("pc").getDisplay().getDisplayMode() == 0 ? null : "GUI still shown",
                        "the console is shown", 5_000)
                .send("pc", "echo console-ok")
                .expect("pc", "^console-ok", "the console takes keys again")
                .await(r -> ui(r).isRunning() && mapped(r).size() == 3 ? null : "GUI lost windows", "the GUI still runs its three apps", 1_000)
                .mutate(r -> chord(r), "press Ctrl+Alt+Backspace again")
                .await(r -> ui(r).guiVisible() ? null : "console still shown", "the GUI is shown again", 5_000)
                .await(r -> drawing(r, "gba"), "gba still drawing", 10_000)
                .note("Control: a client that sends garbage loses only its own connection")
                .mutate(r -> {
                    int[] err = new int[1];
                    UiConnection c = comp(r).open(9999, UiProto.Role.APP, err);
                    BROKEN.put(r, c);
                    if (c != null) c.send(new byte[]{(byte) 0xEE, (byte) 0xEE, 0, 0, 0, 0, 0, 0});
                }, "open a UI connection and send an unknown opcode")
                .await(r -> {
                    UiConnection c = BROKEN.get(r);
                    if (c == null || !c.isClosed() || c.closeReason() == null) return "the broken connection is still open";
                    if (c.closeReason().code != UiProto.ErrorCode.UNKNOWN_OPCODE) return "closed with " + c.closeReason().code;
                    return comp(r).shellConnected() ? tiled(r, 3) : "the shell lost its connection too";
                }, "only that connection closed (Error unknown_opcode); the shell and all three apps are fine", 2_000)
                .timeLimit(60_000)
                .build();
    }

    static com.example.evanscomputermod.testing.SceneViewerMirror viewer(ScenarioRun r) {
        return VIEWER.get(r);
    }

    /** Pump the client and compare its display with the compositor's frame. */
    static String clientMatches(ScenarioRun r) {
        var v = viewer(r);
        if (v == null) return "no viewer";
        v.pump();
        if (!v.active()) return "the client doesn't draw the GUI itself";
        return v.compare(comp(r));
    }

    /**
     * gui_client_render (UI_PROTOCOL.md §14 phase 2a): a player whose client
     * draws the GUI itself, played headless with the real hub, packet codec and
     * client code. Its display must equal the server's frame pixel for pixel;
     * typing in Notes must cost it under 1 KB/s; a legacy pixel window streams
     * as per-surface tiles. Controls: a player without the client capability,
     * client rendering switched off, and the console, all go back to tiles.
     */
    private static Scenario guiClientRender() {
        return Scenario.builder("gui_client_render",
                        "the GUI drawn by the client from scene patches: identical pixels, typing under 1 KB/s, fallbacks")
                .host("pc", PC, "-")
                .decor(new Desk(UI_TOML_NOTES))
                .note("Start the GUI: the test shell opens Notes")
                .send("pc", "gui")
                .expect("pc", "^gui: started", "the compositor and the test shell started")
                .await(r -> comp(r) != null && comp(r).shellConnected() ? null : "waiting for the shell", "the test shell connected", 15_000)
                .await(r -> tiled(r, 1), "Notes' window drawn", 30_000)
                .note("A player whose client can draw scenes watches (it said hello with the scene stream version)")
                .mutate(r -> VIEWER.put(r, new com.example.evanscomputermod.testing.SceneViewerMirror(ui(r), r.terminal("pc").getComputerId())),
                        "a scene-capable player opens the Terminal")
                .await(r -> viewer(r).eligible() ? null : "not streamed as a scene", "the server streams the GUI to it as a scene", 1_000)
                .await(r -> ui(r).sceneViewers(0, List.of(java.util.UUID.randomUUID())).isEmpty() ? null : "a client without the hello got scenes",
                        "control: a player whose client never said hello stays on tile deltas", 1_000)
                .await(GuiScenarios::clientMatches, "the client draws the desktop itself, identical to the server's frame", 10_000)
                .note("Type into Notes at about 100 words a minute: the patches cost the player under 1 KB/s")
                .mutate(r -> {
                    Compositor.WindowInfo w = mapped(r).stream().filter(x -> x.appId().equals("notes")).findFirst().orElseThrow();
                    // Notes' text field is its first row (12 lu padding): click into it.
                    int x = w.x() + 40, y = w.y() + 22;
                    comp(r).pointer(UiProto.PointerKind.MOVE, x, y, 0, UiProto.Button.NONE, 0);
                    comp(r).pointer(UiProto.PointerKind.DOWN, x, y, 1, UiProto.Button.LEFT, 0);
                    comp(r).pointer(UiProto.PointerKind.UP, x, y, 0, UiProto.Button.NONE, 0);
                }, "click Notes' text field")
                .await(r -> {
                    viewer(r).pump();
                    return null;
                }, "let the click settle", 1_000)
                .waitMs(500, "the field takes focus")
                .await(r -> {
                    var v = viewer(r);
                    v.pump();
                    long now = System.currentTimeMillis();
                    long[] t = TYPING.computeIfAbsent(r, k -> new long[]{now, v.bytes(), 0, v.counters().steps()});
                    long due = t[0] + t[2] * 1000L / KEYS_PER_SECOND;
                    if (t[2] < KEYS && now >= due) {
                        comp(r).text(String.valueOf("the quick brown fox jumps".charAt((int) t[2])));
                        t[2]++;
                    }
                    // A quiet tail after the last key, so its patch is counted.
                    return t[2] >= KEYS && now >= due + 250 ? null : "typing (" + t[2] + " keys)";
                }, KEYS + " keystrokes at " + KEYS_PER_SECOND + " a second", 10_000)
                .await(r -> {
                    var v = viewer(r);
                    long[] t = TYPING.get(r);
                    double seconds = (double) KEYS / KEYS_PER_SECOND;
                    long bytes = v.bytes() - t[1], steps = v.counters().steps() - t[3];
                    double perS = bytes / seconds;
                    EvansComputerMod.LOGGER.info("[gui_client_render] typing {} keys in Notes: {} scene steps, {} B, {} B/s",
                            KEYS, steps, bytes, Math.round(perS));
                    if (steps < KEYS / 2) return "only " + steps + " patches while typing: the keys didn't reach Notes";
                    return perS < 1024 ? null : "typing costs " + Math.round(perS) + " B/s";
                }, "typing in Notes cost the watching player under 1 KB/s (and did change the scene)", 1_000)
                .await(GuiScenarios::clientMatches, "after typing, the client still shows exactly the server's frame", 5_000)
                .note("A legacy pixel program in a window: its pixels stream as tiles of its own surface")
                .mutate(r -> ui(r).control("launch gba arm.gba"), "gui launch gba arm.gba")
                .await(r -> tiled(r, 2), "Notes and gba tiled", 30_000)
                .await(r -> drawing(r, "gba"), "gba draws the test ROM", 30_000)
                .await(GuiScenarios::clientMatches, "the client shows gba's frames too, identical to the server's", 10_000)
                .note("Control: client rendering switched off (ui.clientRendering = false) goes back to tile deltas")
                .mutate(r -> System.setProperty("ecm.ui.clientRendering", "false"), "switch client rendering off")
                .await(r -> {
                    viewer(r).pump();
                    return viewer(r).active() ? "the client still draws scenes" : null;
                }, "the stream ended (END): the client hands the display back to tiles", 2_000)
                .mutate(r -> System.clearProperty("ecm.ui.clientRendering"), "switch client rendering on again")
                .await(GuiScenarios::clientMatches, "a fresh keyframe and the client draws it again, identical", 10_000)
                .note("Ctrl+Alt+Backspace: the console is pixels again, so the scene stream ends; back to the GUI, it resumes")
                .mutate(r -> chord(r), "press Ctrl+Alt+Backspace")
                .await(r -> {
                    viewer(r).pump();
                    return !ui(r).guiVisible() && !viewer(r).active() ? null : "still streaming a scene";
                }, "console shown, scene stream ended", 5_000)
                .mutate(r -> chord(r), "press Ctrl+Alt+Backspace again")
                .await(GuiScenarios::clientMatches, "the GUI again, drawn by the client, identical", 10_000)
                .mutate(r -> {
                    var v = viewer(r);
                    var c = v.counters();
                    EvansComputerMod.LOGGER.info("[gui_client_render] the player got {} B in {} messages ({} steps, {} keyframes, {} images)",
                            c.bytes(), c.messages(), c.steps(), c.keyframes(), c.resources());
                    v.pump();
                    v.close();
                }, "log what the player was sent")
                .timeLimit(60_000)
                .build();
    }

    /** Ctrl+Alt+Backspace, as the player presses it: switch between the console and the GUI. */
    static void chord(ScenarioRun r) {
        ComputerInstance c = computer(r);
        c.handleKeyEvent(0x2A, UiManager.SWITCH_MODS, true, false);
        c.handleKeyEvent(0x2A, UiManager.SWITCH_MODS, false, false);
    }

    /** The floor, the SDR, /etc/ui.toml and the test ROM on the computer's disk. */
    private static final class Desk implements Scenario.Decor {
        private final List<BlockPos> floor = new ArrayList<>();
        private final String toml;

        Desk(String toml) {
            this.toml = toml;
            for (int x = -3; x <= 3; x++)
                for (int z = -2; z <= 2; z++) floor.add(new BlockPos(x, 0, z));
        }

        @Override
        public List<BlockPos> footprint() {
            List<BlockPos> all = new ArrayList<>(floor);
            all.add(SDR);
            return all;
        }

        @Override
        public void terrain(ScenarioRun run) {
            for (BlockPos p : floor) run.level().setBlock(run.abs(p), Blocks.SMOOTH_STONE.defaultBlockState(), 3);
        }

        @Override
        public void build(ScenarioRun run) {
            run.level().setBlock(run.abs(SDR), RadioSdrContent.SDR_STANDARD.get().defaultBlockState()
                    .setValue(SdrBlock.FACING, Direction.SOUTH), 3);
            TerminalBlockEntity pc = run.terminal("pc");
            if (pc == null) throw new IllegalStateException("no computer at " + run.abs(PC));
            try {
                Path dir = com.example.evanscomputermod.computer.ComputerStorage.path(pc);
                Files.createDirectories(dir.resolve("etc"));
                Files.writeString(dir.resolve("etc/ui.toml"), toml);
                try (InputStream in = GuiScenarios.class.getResourceAsStream("/evanscomputermod/scenarios/gui/arm.gba")) {
                    if (in == null) throw new IllegalStateException("arm.gba is missing from the mod jar");
                    Files.write(dir.resolve("arm.gba"), in.readAllBytes());
                }
            } catch (IOException e) {
                throw new IllegalStateException("can't prepare the computer's disk", e);
            }
        }

        @Override
        public void clear(ScenarioRun run) {
            UiManager u = ui(run);
            if (u != null) u.stop();
        }
    }
}
//?}
