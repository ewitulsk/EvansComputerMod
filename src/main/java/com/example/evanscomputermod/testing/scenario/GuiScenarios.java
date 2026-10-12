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

    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();
    /** When each run killed its shell (for the "back within a second" check). */
    private static final Map<ScenarioRun, Long> KILLED_AT = new WeakHashMap<>();
    private static final Map<ScenarioRun, UiConnection> BROKEN = new WeakHashMap<>();

    static {
        Scenario s = guiBasics();
        ALL.put(s.name, s);
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
                .decor(new Desk())
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

    private static void chord(ScenarioRun r) {
        ComputerInstance c = computer(r);
        c.handleKeyEvent(0x2A, UiManager.SWITCH_MODS, true, false);
        c.handleKeyEvent(0x2A, UiManager.SWITCH_MODS, false, false);
    }

    /** The floor, the SDR, /etc/ui.toml and the test ROM on the computer's disk. */
    private static final class Desk implements Scenario.Decor {
        private final List<BlockPos> floor = new ArrayList<>();

        Desk() {
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
                Files.writeString(dir.resolve("etc/ui.toml"), UI_TOML);
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
