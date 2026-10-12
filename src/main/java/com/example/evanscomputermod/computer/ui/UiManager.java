package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.display.DisplayDevice;
import com.example.evanscomputermod.computer.ui.proto.UiProto;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;

/**
 * The GUI side of one computer (§2 "Console and GUI modes"): starts and stops
 * the Terminal display's {@link Compositor} and its shell, switches the
 * display between the console and the GUI (both keep running; only the
 * visible one gets input), reads {@code /etc/ui.toml}, gives legacy
 * programs pixel windows, mirrors the GUI to an attached Screen cluster, and
 * is the compositor's {@link CompositorHost}.
 *
 * <p><b>Threads.</b> Compositor ticks run on one shared {@code ECM-UI}
 * thread for all computers, paced to the display refresh. A tick composites
 * the damaged rectangles into the compositor's frame and
 * {@link #present} copies exactly those rectangles into the Terminal's
 * {@link TerminalDisplay} (RGBA8888, mode 1), marks them for the sync path
 * and asks for a display sync, which streams the frame through the existing
 * {@code DisplaySyncHub} like any program frame.
 */
public final class UiManager implements CompositorHost {
    /** Ctrl+Alt+Backspace switches between the console and the GUI. */
    public static final int SWITCH_MODS = UiProto.Mods.CTRL | UiProto.Mods.ALT;

    private static final ScheduledExecutorService TICKS = Executors.newSingleThreadScheduledExecutor(r -> {
        Thread t = new Thread(r, "ECM-UI");
        t.setDaemon(true);
        return t;
    });
    private static final long T0 = System.nanoTime();

    private final ComputerInstance computer;
    private final java.util.function.IntSupplier maxDisplayBytes;
    private final DisplayDevice.RefreshLimits refreshLimits;
    private volatile Compositor compositor;
    private volatile boolean guiVisible;
    private volatile UiConfig config = UiConfig.defaults();
    private final Map<Integer, LegacyWindow> legacy = new ConcurrentHashMap<>();
    /** Programs the compositor started (shell, Launch): their gfx always goes to a window. */
    private final java.util.Set<Integer> launched = ConcurrentHashMap.newKeySet();
    /** The console's frame while the GUI is shown: programs drawing on the console keep a target. */
    private final TerminalDisplay consoleShadow = new TerminalDisplay(1, 1);
    private final Object tickLock = new Object();
    /** The answer to the last {@code gui prompt} (diagnostics). */
    private volatile String lastPermissionAnswer = "";
    private long tickDueNanos = Long.MAX_VALUE;

    public UiManager(ComputerInstance computer, java.util.function.IntSupplier maxDisplayBytes,
                     DisplayDevice.RefreshLimits refreshLimits) {
        this.computer = computer;
        this.maxDisplayBytes = maxDisplayBytes;
        this.refreshLimits = refreshLimits;
    }

    // ================================================================= modes

    public boolean isRunning() {
        return compositor != null;
    }

    /** Is the GUI on screen (it owns the Terminal display and its input)? */
    public boolean guiVisible() {
        return guiVisible && compositor != null;
    }

    public Compositor compositor() {
        return compositor;
    }

    /** After boot: start the GUI if {@code /etc/ui.toml} says {@code autostart = true}. */
    public void onBoot() {
        config = UiConfig.load(configPath());
        if (config.autostart()) {
            TICKS.schedule(() -> {
                try {
                    start();
                } catch (Throwable t) {
                    EvansComputerMod.LOGGER.error("GUI autostart failed", t);
                }
            }, 500, TimeUnit.MILLISECONDS);
        }
    }

    private java.nio.file.Path configPath() {
        java.nio.file.Path root = computer.storagePath();
        return root == null ? null : root.resolve("etc").resolve("ui.toml");
    }

    /** Start the compositor (if it isn't running) with the configured shell, and show it. */
    public synchronized String start() {
        if (compositor == null) {
            config = UiConfig.load(configPath());
            Compositor c = new Compositor(this, currentDisplay(), UiLimits.fromConfig(),
                    com.example.evanscomputermod.EcmConfig.displayDefaultRefreshHz(), config.shell(), config.shellArgs(),
                    config.theme());
            c.setPreTick(() -> {
                for (LegacyWindow w : legacy.values()) w.pull(c);
            });
            compositor = c;
            setVisible(true);
            c.startShell();
            return "gui: started (shell " + config.shell() + ")";
        }
        setVisible(true);
        return "gui: running";
    }

    /** The Terminal display for the installed panel (Tier 2 when the computer has none to report). */
    private UiDisplay currentDisplay() {
        var tier = computer.displayTier(com.example.evanscomputermod.api.IComputerHost.DISPLAY_OWN);
        return tier == null ? UiDisplay.terminal() : UiDisplay.forTier(tier, com.example.evanscomputermod.computer.ui.proto.UiProto.Device.TERMINAL);
    }

    /** The panel changed (hot-plug, §4): the shell gets DisplayChanged and re-places windows. */
    public void panelChanged() {
        Compositor c = compositor;
        if (c == null) return;
        UiDisplay d = currentDisplay();
        if (!d.equals(c.display())) c.setDisplay(d);
    }

    /** Close the compositor: windows go away, apps see their connection closed; the console comes back. */
    public synchronized String stop() {
        Compositor c = compositor;
        if (c == null) return "gui: not running";
        setVisible(false);
        compositor = null;
        int shellPid = c.close();
        if (shellPid > 0) killProgram(shellPid);
        for (LegacyWindow w : new ArrayList<>(legacy.values())) w.device.releaseAll();
        legacy.clear();
        return "gui: stopped";
    }

    /** Show the GUI ({@code true}) or the console. Both keep running. */
    public synchronized void setVisible(boolean v) {
        Compositor c = compositor;
        if (c == null) v = false;
        if (guiVisible == v) return;
        guiVisible = v;
        if (c != null) c.setVisible(v);
        if (!v) computer.onGuiHidden();
        EvansComputerMod.LOGGER.info("Computer {}: {} visible", computer.idString(), v ? "GUI" : "console");
    }

    /** The console/GUI switch chord. */
    public synchronized void toggle() {
        if (compositor != null) setVisible(!guiVisible);
    }

    /** {@code /dev/uictl} commands (see {@link UiCtlFd}). */
    public String control(String line) {
        String[] parts = line.trim().split("\\s+");
        String cmd = parts.length == 0 ? "" : parts[0];
        switch (cmd) {
            case "start", "" -> {
                return start() + "\n";
            }
            case "stop" -> {
                return stop() + "\n";
            }
            case "console" -> {
                setVisible(false);
                return compositor == null ? "gui: not running\n" : "gui: console shown\n";
            }
            case "show" -> {
                if (compositor == null) return "gui: not running (use gui start)\n";
                setVisible(true);
                return "gui: shown\n";
            }
            case "toggle" -> {
                toggle();
                return "gui: " + (guiVisible() ? "shown" : "console shown") + "\n";
            }
            case "launch" -> {
                if (parts.length < 2) return "usage: gui launch PROGRAM [ARGS...]\n";
                if (compositor == null) start();
                List<String> args = new ArrayList<>(List.of(parts).subList(2, parts.length));
                int pid = spawn(parts[1], args);
                return pid > 0 ? "gui: launched " + parts[1] + " (pid " + pid + ")\n" : "gui: can't start " + parts[1] + "\n";
            }
            case "status" -> {
                return status();
            }
            case "prompt" -> {
                // Show the shell's permission dialog (P4 routes real prompts this way); the answer is logged.
                Compositor c = compositor;
                if (c == null) return "gui: not running\n";
                if (parts.length < 3) return "usage: gui prompt APP_ID PERMISSION\n";
                String app = parts[1], perm = parts[2];
                int req = c.requestPermission(app, perm, (allow, remember) -> {
                    lastPermissionAnswer = app + " " + perm + ": " + (allow ? "allowed" : "denied") + (remember ? " (remembered)" : "");
                    log("permission " + lastPermissionAnswer);
                });
                return "gui: asked the shell (request " + req + ")\n";
            }
            default -> {
                return "gui: unknown command '" + cmd + "' (start, stop, console, show, toggle, launch, status, prompt)\n";
            }
        }
    }

    public String status() {
        Compositor c = compositor;
        if (c == null) return "gui: not running\n";
        StringBuilder sb = new StringBuilder();
        sb.append("gui: running, ").append(guiVisible() ? "shown" : "console shown")
                .append(", shell ").append(config.shell()).append(c.shellConnected() ? " connected" : " not connected")
                .append(c.safeMode() ? ", SAFE MODE (" + c.safeReason() + ")" : "")
                .append(", ").append(c.windows().size()).append(" windows")
                .append(lastPermissionAnswer.isEmpty() ? "" : ", last permission answer: " + lastPermissionAnswer).append('\n');
        for (Compositor.WindowInfo w : c.windows()) {
            sb.append("  window ").append(w.id()).append(" pid ").append(w.pid()).append(' ').append(w.appId())
                    .append(" \"").append(w.title()).append("\" ").append(w.x()).append(',').append(w.y())
                    .append(' ').append(w.w()).append('x').append(w.h())
                    .append(w.mapped() ? "" : " (not drawn yet)").append(w.focused() ? " focused" : "")
                    .append(w.legacy() ? " legacy" : "").append(w.shell() ? " (shell)" : "").append('\n');
        }
        return sb.toString();
    }

    // ================================================================= connections

    /** {@code ui_open}: -4 in console mode (no compositor), -1 for a bad role/version, -2 shell taken. */
    public Object open(int pid, int role, int version) {
        Compositor c = compositor;
        if (c == null) return UiConnection.E_NO_DEVICE;
        if (version != UiProto.VERSION || (role != 0 && role != 1)) return UiConnection.E_INVALID;
        int[] err = new int[1];
        UiConnection conn = c.open(pid, role == 1 ? UiProto.Role.SHELL : UiProto.Role.APP, err);
        return conn != null ? conn : (Object) err[0];
    }

    // ================================================================= input

    /**
     * A key from the player (HID usage, protocol mods). The switch chord
     * works in both modes; otherwise keys go to the GUI only while it is shown.
     * Returns true if the key was used here (the console must not see it).
     */
    public boolean key(int hid, int mods, boolean down, boolean repeat) {
        if (down && hid == HidKeys.BACKSPACE && (mods & SWITCH_MODS) == SWITCH_MODS && compositor != null) {
            if (!repeat) toggle();
            return true;
        }
        Compositor c = compositor;
        if (c == null || !guiVisible) return false;
        c.key(hid, mods, down, repeat);
        return true;
    }

    public void text(String s) {
        Compositor c = compositor;
        if (c != null && guiVisible) c.text(s);
    }

    /** A console-style key string while the GUI is shown: a focused legacy window gets it on stdin. */
    public void legacyText(String s) {
        Compositor c = compositor;
        if (c == null || !guiVisible) return;
        if (c.focusedLegacy() instanceof LegacyWindow w) {
            computer.getProcessManager().writeInput(w.pid, s.getBytes(StandardCharsets.UTF_8));
        }
    }

    /** A pointer event in Terminal framebuffer pixels (MouseInputPacket kinds and GLFW buttons). */
    public void pointer(int kind, int x, int y, int held, int glfwButton, int scrollDir) {
        Compositor c = compositor;
        if (c == null || !guiVisible) return;
        UiProto.PointerKind k = switch (kind) {
            case 2 -> UiProto.PointerKind.DOWN;
            case 3 -> UiProto.PointerKind.UP;
            case 4 -> UiProto.PointerKind.SCROLL;
            default -> UiProto.PointerKind.MOVE;
        };
        UiProto.Button b = UiProto.Button.NONE;
        if (k == UiProto.PointerKind.DOWN || k == UiProto.PointerKind.UP) {
            b = switch (glfwButton) {
                case 0 -> UiProto.Button.LEFT;
                case 1 -> UiProto.Button.RIGHT;
                case 2 -> UiProto.Button.MIDDLE;
                default -> UiProto.Button.NONE;
            };
        }
        c.pointer(k, x, y, held & UiProto.Buttons.MASK, b, k == UiProto.PointerKind.SCROLL ? -scrollDir : 0);
    }

    // ================================================================= legacy windows

    /**
     * The display device a program's gfx calls on the Terminal use: its own
     * window's while the GUI runs and it either was started by the GUI or the
     * GUI is on screen (or it already has a window); null means the Terminal's
     * own device (console mode, exactly as before).
     */
    public DisplayDevice programDevice(int pid) {
        if (pid <= 0) return null;
        LegacyWindow w = legacy.get(pid);
        if (w != null) return w.device;
        if (compositor == null || !(guiVisible || launched.contains(pid))) return null;
        String name = computer.getProcessManager().nameOf(pid);
        LegacyWindow nw = new LegacyWindow(this, pid, name == null ? "program " + pid : stripWasm(name),
                maxDisplayBytes, refreshLimits);
        legacy.put(pid, nw);
        return nw.device;
    }

    /** The legacy window device a program already has, or null. */
    public DisplayDevice existingDevice(int pid) {
        LegacyWindow w = legacy.get(pid);
        return w == null ? null : w.device;
    }

    void legacyDirty() {
        requestTick(0);
    }

    void legacyReleased(LegacyWindow w) {
        legacy.remove(w.pid, w);
        Compositor c = compositor;
        if (c != null && w.conn != null) c.disconnect(w.conn, null);
    }

    /** {@code mouse_capture_start} from a program with a window: 1, or -1 if it has none. */
    public int mouseCaptureStart(int pid) {
        LegacyWindow w = legacy.get(pid);
        if (w == null) return -1;
        w.capture = true;
        return 1;
    }

    public boolean mouseCaptureStop(int pid) {
        LegacyWindow w = legacy.get(pid);
        if (w == null) return false;
        w.stopCapture();
        return true;
    }

    /** 1 / 0 for a program with a window, -1 otherwise. */
    public int mouseCaptureActive(int pid) {
        LegacyWindow w = legacy.get(pid);
        return w == null ? -1 : (w.capture ? 1 : 0);
    }

    /** {@code mouse_poll} for a program with a window: 1 / 0, or -1 if it has none. */
    public int mousePoll(int pid, byte[] out) {
        LegacyWindow w = legacy.get(pid);
        return w == null ? -1 : w.pollMouse(out);
    }

    void killProgram(int pid) {
        computer.getProcessManager().kill(pid);
    }

    /** A program exited: its window and connections go away; a shell exit starts safe mode. */
    public void onProcessExit(int pid) {
        launched.remove(pid);
        LegacyWindow w = legacy.get(pid);
        if (w != null) w.device.release(pid);
        Compositor c = compositor;
        if (c != null) c.processExited(pid);
    }

    /** The scanout the console's own display device draws into: the Terminal, or a shadow while the GUI is shown. */
    public TerminalDisplay consoleScanout(TerminalDisplay real) {
        return guiVisible() ? consoleShadow : real;
    }

    public void shutdown() {
        stop();
    }

    // ================================================================= CompositorHost

    @Override
    public int spawn(String program, List<String> args) {
        int pid = computer.spawnDetached(program, args);
        if (pid <= 0 && program.contains(".")) {
            pid = computer.spawnDetached(program.substring(program.lastIndexOf('.') + 1), args);
        }
        if (pid > 0) launched.add(pid);
        return pid;
    }

    @Override
    public void kill(int pid) {
        killProgram(pid);
    }

    @Override
    public String appId(int pid) {
        String n = computer.getProcessManager().nameOf(pid);
        return n == null ? "pid-" + pid : stripWasm(n);
    }

    private static String stripWasm(String n) {
        return n.endsWith(".wasm") ? n.substring(0, n.length() - 5) : n;
    }

    @Override
    public void present(byte[] rgba, int w, int h, List<Box> damage) {
        if (!guiVisible) return;
        TerminalDisplay td = computer.terminalDisplay();
        if (td != null) scanOut(td, rgba, w, h, damage);
        if ("mirror".equals(config.screenMode())) {
            TerminalDisplay sd = computer.mirrorTarget();
            if (sd != null) scanOut(sd, rgba, w, h, damage);
        }
        computer.requestUiSync();
    }

    private static void scanOut(TerminalDisplay td, byte[] rgba, int w, int h, List<Box> damage) {
        synchronized (td) {
            boolean rects = true;
            for (Box b : damage) {
                if (!td.setGfxRect(1, w, h, TerminalDisplay.PIXEL_FORMAT_RGBA8888, rgba, b.x0(), b.y0(), b.w(), b.h())) {
                    rects = false;
                    break;
                }
            }
            if (!rects) {
                td.setGfxFrame(1, w, h, TerminalDisplay.PIXEL_FORMAT_RGBA8888, null, rgba);
            } else {
                for (Box b : damage) td.addSyncDamage(b.x0(), b.y0(), b.w(), b.h());
            }
            if (td.getDisplayMode() != 1) td.setDisplayMode(1);
        }
    }

    @Override
    public void requestTick(long delayNanos) {
        Compositor c = compositor;
        if (c == null) return;
        long due = System.nanoTime() + delayNanos;
        synchronized (tickLock) {
            if (tickDueNanos <= due && tickDueNanos > System.nanoTime() - 50_000_000L) return;
            tickDueNanos = due;
        }
        TICKS.schedule(() -> {
            synchronized (tickLock) {
                if (tickDueNanos == due) tickDueNanos = Long.MAX_VALUE;
            }
            Compositor cc = compositor;
            if (cc == null) return;
            try {
                cc.tick();
            } catch (Throwable t) {
                EvansComputerMod.LOGGER.error("Compositor tick failed", t);
            }
        }, Math.max(0, delayNanos), TimeUnit.NANOSECONDS);
    }

    @Override
    public long nowMs() {
        return (System.nanoTime() - T0) / 1_000_000L;
    }

    @Override
    public void log(String msg) {
        EvansComputerMod.LOGGER.info("[gui {}] {}", computer.idString(), msg);
    }
}
