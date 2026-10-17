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
    /**
     * App-manager work the compositor must not wait for under its lock: an
     * {@code Open{path}} (may launch an app or ask about a file permission) and
     * the {@code notifications} check of {@code Notify} (may show a prompt).
     */
    private static final java.util.concurrent.ExecutorService SERVICES = Executors.newCachedThreadPool(r -> {
        Thread t = new Thread(r, "ECM-UI-Services");
        t.setDaemon(true);
        return t;
    });

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
    /** Client rendering (§14 phase 2a): the scene stream to players who draw the GUI themselves. */
    private final UiSceneHub sceneHub = new UiSceneHub();
    private static final java.util.concurrent.atomic.AtomicInteger EPOCHS =
            new java.util.concurrent.atomic.AtomicInteger((int) (System.nanoTime() & 0xFFFF) << 8);
    /** An ePhone's GUI side (P5): set at boot on a phone, which is always in GUI mode; null on Terminals. */
    private volatile PhoneUi phone;

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

    /**
     * After boot: start the GUI if {@code /etc/ui.toml} says {@code autostart = true}.
     * An ePhone always starts it, with the phone shell (P5).
     */
    public void onBoot() {
        if (computer.isPhone() && phone == null) {
            phone = new PhoneUi(computer, this);
            phone.boot();
        }
        config = loadConfig();
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
            config = loadConfig();
            Compositor c = new Compositor(this, currentDisplay(), UiLimits.fromConfig(),
                    com.example.evanscomputermod.EcmConfig.displayDefaultRefreshHz(), config.shell(), config.shellArgs(),
                    config.theme());
            c.setPreTick(() -> {
                for (LegacyWindow w : legacy.values()) w.pull(c);
            });
            compositor = c;
            installAppHooks(c);
            setVisible(true);
            c.startShell();
            return "gui: started (shell " + config.shell() + ")";
        }
        setVisible(true);
        return "gui: running";
    }

    /** {@code /etc/ui.toml}; on a phone with the phone shell pinned and autostart on (PhoneUi#config). */
    private UiConfig loadConfig() {
        UiConfig c = UiConfig.load(configPath());
        PhoneUi p = phone;
        return p != null ? p.config(c) : c;
    }

    /**
     * The display for the installed panel: a Terminal's landscape mode, an
     * ePhone's portrait one (Tier 2 Terminal when the computer has none to report).
     */
    private UiDisplay currentDisplay() {
        var tier = computer.displayTier(com.example.evanscomputermod.api.IComputerHost.DISPLAY_OWN);
        if (tier == null) {
            return phone != null ? UiDisplay.forTier(com.example.evanscomputermod.computer.display.PanelTier.COLOR_LCD, UiProto.Device.EPHONE)
                    : UiDisplay.terminal();
        }
        return UiDisplay.forTier(tier, phone != null ? UiProto.Device.EPHONE : UiProto.Device.TERMINAL);
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
        removeAppHooks();
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

    /** The console/GUI switch chord. A phone has no console screen: it stays on the GUI. */
    /** The desktop's Console item: show the console (the GUI keeps running). */
    @Override
    public void showConsole() {
        if (phone == null) setVisible(false);
    }

    @Override
    public void clipboardChanged(String mime, byte[] data) {
        if (mime != null && mime.startsWith("text/")) computer.clipboardToPlayer(new String(data, StandardCharsets.UTF_8));
    }

    /** The player's real clipboard, right before their paste key. */
    public void clipboardFromPlayer(String text) {
        Compositor c = compositor;
        if (c != null) c.setClipboardText(text);
    }

    public synchronized void toggle() {
        if (compositor != null && phone == null) setVisible(!guiVisible);
    }

    /**
     * {@code /dev/uictl} from program {@code pid}: the P4 dialog answers
     * ({@code picked TOKEN PATH|cancel}, {@code chosen TOKEN APP_ID [always]|cancel},
     * accepted only from the picker / chooser started for TOKEN), and every
     * other command as {@link #control(String)}.
     */
    public String control(int pid, String line) {
        String t = line.trim();
        if (t.startsWith("picked ") || t.startsWith("chosen ")) {
            Compositor c = compositor;
            return c == null ? "gui: not running\n" : c.systemAnswer(pid, t);
        }
        // Apps (confined pids) may only launch, and what they launch can't escape their sandbox
        // (see launch): they can't stop the GUI, switch it away or ask prompts.
        var apps = computer.getAppManager();
        if (apps != null && apps.isConfined(pid)) {
            String cmd = t.isEmpty() ? "" : t.split("\\s+")[0];
            if (!cmd.equals("launch")) return "gui: apps may only use launch\n";
        }
        return command(pid, line);
    }

    /** {@code /dev/uictl} commands from the system (see {@link UiCtlFd}). */
    public String control(String line) {
        return command(0, line);
    }

    private String command(int pid, String line) {
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
                int started = launch(pid, parts[1], args);
                return started > 0 ? "gui: launched " + parts[1] + " (pid " + started + ")\n" : "gui: can't start " + parts[1] + "\n";
            }
            case "status" -> {
                return status();
            }
            case "phone" -> {
                PhoneUi p = phone;
                return p == null ? "gui: not a phone\n" : p.statusText();
            }
            case "cell" -> {
                // celld's report for the status bar: cell <bars 0-4> <carrier|->
                PhoneUi p = phone;
                if (p == null) return "gui: not a phone\n";
                if (parts.length < 2) return "usage: cell BARS [CARRIER]\n";
                try {
                    p.cellReport(Integer.parseInt(parts[1]), parts.length > 2 ? parts[2] : "-");
                } catch (NumberFormatException e) {
                    return "usage: cell BARS [CARRIER]\n";
                }
                return "ok\n";
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
                return "gui: unknown command '" + cmd + "' (start, stop, console, show, toggle, launch, status, phone, cell, prompt)\n";
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
        PhoneUi p = phone;
        if (p != null) p.close();
        stop();
    }

    // ================================================================= client rendering

    /** The scene stream's hub (bandwidth counters, tests). */
    public UiSceneHub sceneHub() {
        return sceneHub;
    }

    /**
     * Does display {@code displayId} show the GUI scene right now, so players
     * who can may draw it themselves? The Terminal's own display while the
     * GUI is shown; a Screen wall while it mirrors the GUI. Off when
     * {@code ui.clientRendering} is false.
     */
    public boolean sceneEligible(int displayId) {
        if (!com.example.evanscomputermod.EcmConfig.uiClientRendering() || compositor == null || !guiVisible) return false;
        if (displayId == com.example.evanscomputermod.api.IComputerHost.DISPLAY_OWN) return true;
        return displayId == com.example.evanscomputermod.api.IComputerHost.DISPLAY_ATTACHED
                && "mirror".equals(config.screenMode()) && computer.mirrorTarget() != null;
    }

    /** Which of {@code viewers} get display {@code displayId} as a scene (the rest get tile deltas). */
    public List<java.util.UUID> sceneViewers(int displayId, List<java.util.UUID> viewers) {
        if (viewers.isEmpty() || !sceneEligible(displayId)) return List.of();
        List<java.util.UUID> out = new ArrayList<>();
        for (java.util.UUID id : viewers) if (UiClients.canRender(id)) out.add(id);
        return out;
    }

    /**
     * One scene pass for a display (server thread): steps to {@code ids},
     * keyframes to new ones, END to players who no longer draw it. Attaches
     * the exporter to the compositor while anyone renders the scene and
     * detaches it when nobody does.
     */
    public void sceneSync(int displayId, List<java.util.UUID> ids, UiSceneHub.Sink sink) {
        Compositor c = compositor;
        if (c != null && !ids.isEmpty() && c.exporter() == null) {
            int epoch = EPOCHS.incrementAndGet();
            c.setExporter(new SceneExporter(c, epoch, step -> {
                sceneHub.offer(step);
                computer.requestUiSync();
            }));
        }
        sceneHub.sync(displayId, ids, () -> {
            Compositor cc = compositor;
            return cc == null ? null : cc.exporter();
        }, sink);
        if (c != null && !sceneHub.hasViewers() && c.exporter() != null) c.setExporter(null);
    }

    // ================================================================= app manager hooks

    /**
     * The compositor runs: undecided permissions are asked through the shell's
     * dialog ({@link #promptViaShell}), {@code OpenFile} reaches opened apps and
     * {@code WillSuspend} reaches an app being suspended (so its
     * {@code SaveState} can follow). Terminals and other non-phone computers
     * only: an ePhone's PhoneUi installs the phone's prompter and app events
     * (the same delivery plus the phone's lifecycle) at boot.
     */
    private void installAppHooks(Compositor c) {
        var apps = computer.getAppManager();
        // Phones install their own prompter and app events (PhoneUi): never override them.
        if (apps == null || computer.isPhone()) return;
        apps.setPrompter(this::promptViaShell);
        apps.setAppEvents(new com.example.evanscomputermod.computer.apps.AppEvents() {
            @Override
            public boolean willSuspend(com.example.evanscomputermod.computer.apps.AppInstance app, int deadlineMs) {
                return deliverWillSuspend(app, deadlineMs);
            }

            @Override
            public void openFile(com.example.evanscomputermod.computer.apps.AppInstance app, String path) {
                deliverOpenFile(app, path);
            }
        });
    }

    /**
     * The one permission prompter path (desktop and phone): the shell's dialog
     * through {@link Compositor#requestPermission}. Null without a running
     * compositor and connected shell, so the app manager uses its console policy.
     */
    java.util.concurrent.CompletableFuture<com.example.evanscomputermod.computer.apps.PermissionPrompter.Answer> promptViaShell(
            com.example.evanscomputermod.computer.apps.PermissionPrompter.Request req) {
        var apps = computer.getAppManager();
        Compositor cc = compositor;
        if (apps == null || cc == null || !cc.shellConnected()) return null;
        var f = new java.util.concurrent.CompletableFuture<com.example.evanscomputermod.computer.apps.PermissionPrompter.Answer>();
        log("permission prompt: " + req.appName() + " (" + req.appId() + ", pid " + req.pid() + ") asks for " + req.permission());
        cc.requestPermission(req.appId(), req.permission(), (allow, remember) -> {
            lastPermissionAnswer = req.appId() + " " + req.permission() + ": " + (allow ? "allowed" : "denied") + (remember ? " (remembered)" : "");
            log("permission " + lastPermissionAnswer);
            // A remembered answer counts even when the app stopped waiting (prompt timeout).
            if (remember) apps.permissions().setDecision(req.appId(), req.permission(), allow);
            f.complete(new com.example.evanscomputermod.computer.apps.PermissionPrompter.Answer(allow, remember));
        });
        return f;
    }

    /** {@code WillSuspend{deadline_ms}} to every UI connection of {@code app}; false if it has none. */
    boolean deliverWillSuspend(com.example.evanscomputermod.computer.apps.AppInstance app, int deadlineMs) {
        var apps = computer.getAppManager();
        Compositor cc = compositor;
        if (apps == null || cc == null) return false;
        return cc.sendToApp(pid -> apps.instanceOf(pid) == app, new UiProto.WillSuspend(deadlineMs)) > 0;
    }

    /** {@code OpenFile{path}} to {@code app} (now, or right after its Welcome). */
    void deliverOpenFile(com.example.evanscomputermod.computer.apps.AppInstance app, String path) {
        Compositor cc = compositor;
        if (cc != null) cc.deliverOpenFile(app.pid(), path);
    }

    private void removeAppHooks() {
        var apps = computer.getAppManager();
        if (apps == null || computer.isPhone()) return;
        apps.setPrompter(null);
        apps.setAppEvents(null);
    }

    // ================================================================= CompositorHost

    @Override
    public int spawn(String program, List<String> args) {
        // Built-in programs (the shell, system dialogs): apps start through launch().
        int pid = computer.spawnDetached(program, args);
        if (pid <= 0 && program.contains(".")) {
            pid = computer.spawnDetached(program.substring(program.lastIndexOf('.') + 1), args);
        }
        if (pid > 0) launched.add(pid);
        return pid;
    }

    /**
     * {@code Launch{app_id}} (shell, desktop launcher, apps) and {@code gui launch}:
     * an installed app starts through the app manager (its own identity and
     * sandbox); anything else is a built-in program, started unconfined for the
     * shell and the system, or into the caller's app group when an app asks
     * (it keeps that app's identity and permissions: no escape through Launch
     * or {@code /dev/uictl launch}).
     */
    @Override
    public int launch(int fromPid, String appId, List<String> args) {
        var apps = computer.getAppManager();
        if (apps != null && appId != null && apps.manifest(appId) != null) {
            PhoneUi p = phone;
            if (p != null) {
                // A phone runs one instance per app (a running one is reused) and keeps its session list.
                int pid = p.launch(appId, args);
                if (pid > 0) launched.add(pid);
                return pid;
            }
            try {
                var inst = apps.launch(appId, args);
                launched.add(inst.pid());
                return inst.pid();
            } catch (com.example.evanscomputermod.computer.apps.AppException e) {
                log("launch " + appId + ": " + e.getMessage());
                return -1;
            }
        }
        int group = apps == null || fromPid <= 0 ? 0 : apps.groupForSpawn(fromPid);
        if (group <= 0) return spawn(appId, args);
        int pid = computer.spawnDetached(appId, args, group);
        if (pid <= 0 && appId.contains(".")) pid = computer.spawnDetached(appId.substring(appId.lastIndexOf('.') + 1), args, group);
        if (pid > 0) {
            launched.add(pid);
            log("launch " + appId + " for app " + apps.appIdOf(fromPid) + ": in its group (pid " + pid + ")");
        }
        return pid;
    }

    /**
     * {@code Open{path}} from Files or an app: the app manager opens it with the
     * default handler (an app gets a one-file grant and {@code OpenFile}; a
     * built-in type starts its program); several handlers and no default show
     * the chooser. Runs off the compositor's lock.
     */
    @Override
    public void open(int fromPid, String path) {
        var apps = computer.getAppManager();
        if (apps == null) {
            log("open " + path + ": no app manager");
            return;
        }
        SERVICES.execute(() -> {
            try {
                var o = apps.openAny(path, fromPid);
                launched.add(o.pid());
                log("opened " + path + " with " + o.handler() + " (pid " + o.pid() + ")");
            } catch (com.example.evanscomputermod.computer.apps.AppException e) {
                if (e.code == com.example.evanscomputermod.computer.apps.AppException.CHOOSE) {
                    Compositor c = compositor;
                    if (c == null || !c.startChooser(fromPid, path)) log("open " + path + ": can't show the chooser");
                } else {
                    log("open " + path + ": " + e.getMessage());
                }
            } catch (RuntimeException e) {
                log("open " + path + " failed: " + e);
            }
        });
    }

    /** {@code Notify} needs {@code notifications} for apps (may prompt, so off the compositor's lock). */
    @Override
    public void checkNotify(int pid, java.util.function.Consumer<Boolean> then) {
        var apps = computer.getAppManager();
        if (apps == null || !apps.isConfined(pid)) {
            then.accept(true);
            return;
        }
        SERVICES.execute(() -> {
            boolean ok;
            try {
                ok = apps.check(pid, "notifications");
            } catch (RuntimeException e) {
                ok = false;
            }
            then.accept(ok);
        });
    }

    @Override
    public void kill(int pid) {
        killProgram(pid);
    }

    /**
     * P4: the picker's answer for program {@code pid}: a one-file grant for its
     * app. Never the app database, nor another app's data.
     */
    @Override
    public void filePicked(int pid, String path, boolean write) {
        var apps = computer.getAppManager();
        if (apps == null) return;
        String p = com.example.evanscomputermod.computer.wasi.ProcessSandbox.normalize(path);
        String id = apps.appIdOf(pid);
        if (p == null || p.startsWith("/var/lib/apps") || (p.startsWith("/var/apps/") && (id == null || !(p + "/").startsWith("/var/apps/" + id + "/")))) {
            log("file picker: refused to grant " + path);
            return;
        }
        apps.onFilePicked(pid, p, write);
    }

    /** P4: the chooser's answer: open the file with that app (and remember it as the default). */
    @Override
    public void openWith(String path, String appId, boolean always) {
        var apps = computer.getAppManager();
        if (apps == null) return;
        try {
            if (always) {
                String name = path.substring(path.lastIndexOf('/') + 1);
                int dot = name.lastIndexOf('.');
                if (dot > 0) apps.setDefaultHandler(name.substring(dot), appId);
            }
            apps.openWith(path, appId);
        } catch (com.example.evanscomputermod.computer.apps.AppException e) {
            log("open with " + appId + ": " + e.getMessage());
        }
    }

    /** The app manager's app id for app processes (any member of the app); the program name otherwise. */
    @Override
    public String appId(int pid) {
        var apps = computer.getAppManager();
        String id = apps == null ? null : apps.appIdOf(pid);
        if (id != null) return id;
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

    static void scanOut(TerminalDisplay td, byte[] rgba, int w, int h, List<Box> damage) {
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

    // ================================================================= app platform wiring (P4/P5)
    //
    // For every computer with the GUI (Terminal and ePhone): SaveState goes to the
    // app manager (AppManager.onSaveState), an app's saved state comes back as
    // Restore right after Welcome (restore_follows), BackgroundDone ends a refresh
    // wake (AppManager.onBackgroundDone). Non-apps' SaveState is dropped.

    @Override
    public byte[] restoreFor(int pid) {
        var apps = computer.getAppManager();
        String id = apps == null ? null : apps.appIdOf(pid);
        if (id == null || !apps.peekRestore(id)) return null;
        return apps.takeRestore(id);
    }

    @Override
    public boolean saveState(int pid, byte[] data) {
        var apps = computer.getAppManager();
        return apps != null && apps.onSaveState(pid, data);
    }

    @Override
    public void backgroundDone(int pid) {
        var apps = computer.getAppManager();
        if (apps != null) apps.onBackgroundDone(pid);
    }

    @Override
    public void focusChanged(int oldPid, int newPid) {
        PhoneUi p = phone;
        if (p != null) p.focusChanged(oldPid, newPid);
    }

    // ================================================================= ePhone (P5)

    /** The phone side of this computer's GUI, or null on a Terminal. */
    public PhoneUi phone() {
        return phone;
    }
}
