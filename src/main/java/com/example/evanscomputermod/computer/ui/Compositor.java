package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.ui.SceneRenderer.Geo;
import com.example.evanscomputermod.computer.ui.SceneRenderer.Item;
import com.example.evanscomputermod.computer.ui.font.UiFonts;
import com.example.evanscomputermod.computer.ui.proto.UiProto;
import com.example.evanscomputermod.computer.ui.proto.UiProto.Opcode;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * The compositor of one display (docs/gui/UI_PROTOCOL.md §1-§9): it owns the
 * UI connections, the scene (layers background / bottom / windows / top /
 * popups / overlay), enforces the protocol and its limits, routes input,
 * runs shell transitions and frame pacing at the display's refresh rate,
 * keeps a built-in <i>safe mode</i> for when the shell is gone, and
 * rasterizes the damaged parts of the scene into one frame.
 *
 * <p><b>Threads.</b> Every method takes the compositor's monitor. Program
 * threads come in through {@link UiConnection} (decode outside the lock,
 * handle inside); player input arrives from the server thread; the
 * {@link CompositorHost}'s scheduler calls {@link #tick()} (on the shared
 * {@code ECM-UI} thread in game), which applies deferred commits, advances
 * animations, sends {@code Frame} events, watches the shell and composites.
 * Compositing writes only into the compositor's own frame;
 * {@link CompositorHost#present} then copies the damaged rectangles to the
 * display, outside of any program lock.
 */
public final class Compositor {
    /** Height of safe mode's text bar, in lu. */
    public static final int SAFE_BAR_H = 14;
    /**
     * Shell restart delays after it stops: 0.5 s (so it is back within a second,
     * ROADMAP P2), then 5 s, then every 30 s (§8 "Safe mode").
     */
    static final long[] RESTART_BACKOFF_MS = {500, 5000, 30000};
    /** A shell that ran this long before stopping restarts after the shortest delay again. */
    static final long STABLE_SHELL_MS = 60_000;
    /** HID usages the compositor itself knows. */
    public static final int HID_TAB = 0x2B, HID_ESCAPE = 0x29, HID_BACKSPACE = 0x2A;
    private static final int BG = SceneRenderer.BG;

    final CompositorHost host;
    final UiLimits limits;
    UiDisplay display;
    private final int refreshHz;
    private final String shellProgram;
    private final List<String> shellArgs;

    final Map<Integer, UiConnection> conns = new LinkedHashMap<>();
    private int nextConn = 1;
    private final Map<Integer, UiWindow> windows = new LinkedHashMap<>();
    /** Windows bottom to top. */
    final List<UiWindow> stack = new ArrayList<>();
    private int nextWindow = 1;
    final List<Surface> layers = new ArrayList<>();
    final List<Surface> popups = new ArrayList<>();
    UiWindow focused;

    // Shell and safe mode.
    private UiConnection shell;
    private int shellPid = -1;
    private long shellStartedMs;
    private int shellStarts;
    private long shellConnectedMs;
    boolean safeMode;
    private String safeReason = "";
    private int backoff;
    private long restartAtMs = -1;

    // Input.
    private int pointerX = -1, pointerY = -1, buttons;
    private Surface hover, grab;
    private final Set<Integer> heldKeys = new HashSet<>();
    private final Set<Integer> swallowed = new HashSet<>();
    private final Map<Integer, int[]> chords = new LinkedHashMap<>();
    private Surface pointerLocked;

    // Services.
    private UiProto.Theme theme;
    private String clipboardMime = "";
    private byte[] clipboard = new byte[0];
    /** The app connection a paste key or click went to last, and when (§12 clipboard rule). */
    private UiConnection pasteArmed;
    private long pasteArmedAt;
    private int nextNid = 1;
    private final Map<Integer, Object[]> notifications = new HashMap<>();
    /** Notification actions for apps that had quit, delivered when the app connects again. */
    private final Map<String, List<UiProto.NotificationAction>> pendingActions = new HashMap<>();
    /** {@code OpenFile} paths for apps that haven't connected yet, by pid (open with). */
    private final Map<Integer, List<String>> pendingOpenFiles = new HashMap<>();
    private int nextPermissionReq = 1;
    private final Map<Integer, PermissionAnswer> permissionWaiters = new HashMap<>();
    private FilePicker filePicker;

    /** How long after a paste key or click the app may read the clipboard (ms). */
    public static final long PASTE_WINDOW_MS = 2000;
    /** HID usages of the paste keys. */
    public static final int HID_V = 0x19, HID_INSERT = 0x49;

    /** The answer to a permission prompt (§10): the P4 app manager waits on it. */
    public interface PermissionAnswer {
        void answered(boolean allow, boolean remember);
    }

    // ePhone (P5): hotkey claims per connection id (slot 1-6 -> label) and the notifications posted.
    private final Map<Integer, String[]> hotkeyClaims = new HashMap<>();
    private final List<PostedNotification> posted = new ArrayList<>();

    /**
     * A replacement for the system file picker (§12), e.g. for tests. A picker
     * calls {@code answer} once, from any thread, with the chosen path or "".
     */
    public interface FilePicker {
        void pick(String appId, UiProto.PickFile request, java.util.function.Consumer<String> answer);
    }

    // Output.
    private boolean visible;
    private int[] frame;
    private byte[] rgba;
    private final Box.Region damage = new Box.Region();
    private final List<UiConnection> toFail = new ArrayList<>();
    private Runnable preTick = () -> {};
    private boolean closedForGood;
    private long frames;
    /** Client rendering (phase 2a): builds scene patches after each tick, or null. */
    private SceneExporter exporter;
    /** P4: the system file picker and "open with" chooser (programs answering on /dev/uictl). */
    private final SystemDialogs dialogs;

    public Compositor(CompositorHost host, UiDisplay display, UiLimits limits, int refreshHz, String shellProgram,
                      List<String> shellArgs, String themeName) {
        this.host = host;
        this.limits = limits;
        this.refreshHz = Math.max(1, refreshHz);
        this.shellProgram = shellProgram;
        this.shellArgs = shellArgs == null ? List.of() : List.copyOf(shellArgs);
        this.theme = new UiProto.Theme(themeName == null || themeName.isEmpty() ? "ecm-default" : themeName, List.of());
        setDisplayLocked(display);
        this.dialogs = new SystemDialogs(host);
    }

    // =================================================================== lifecycle

    /** Show the composited scene (GUI mode) or stop drawing it (console visible). */
    public synchronized void setVisible(boolean v) {
        if (visible == v) return;
        visible = v;
        if (v) damage.add(displayBox());
        if (!v) releaseInputLocked();
        requestTick(0);
    }

    public synchronized boolean isVisible() {
        return visible;
    }

    /** Called before each tick under the lock (the host pulls legacy frames there). */
    public synchronized void setPreTick(Runnable r) {
        preTick = r == null ? () -> {} : r;
    }

    /** Start the configured shell (at startup, and again after it stops). */
    public synchronized void startShell() {
        if (closedForGood) return;
        restartAtMs = -1;
        if (shellProgram == null || shellProgram.isEmpty()) {
            enterSafeMode("no shell configured", false);
            return;
        }
        // A restarted shell is told so (first argument --restart): it must not redo
        // one-time work such as launching the session's apps again.
        List<String> args = shellStarts++ == 0 ? shellArgs : prepend("--restart", shellArgs);
        int pid = host.spawn(shellProgram, args);
        if (pid <= 0) {
            shellPid = -1;
            enterSafeMode("can't start " + shellProgram, true);
            return;
        }
        shellPid = pid;
        shellStartedMs = host.nowMs();
        host.log("started shell " + shellProgram + " as pid " + pid);
        requestTick(1000);
    }

    private static List<String> prepend(String first, List<String> rest) {
        List<String> out = new ArrayList<>();
        out.add(first);
        out.addAll(rest);
        return out;
    }

    /** A program exited: its connections are closed by its fd table; a shell exit starts safe mode. */
    public synchronized void processExited(int pid) {
        dialogs.processExited(pid);
        pendingOpenFiles.remove(pid);
        for (UiConnection c : new ArrayList<>(conns.values())) {
            if (c.pid == pid) disconnectLocked(c, null);
        }
        if (pid == shellPid) {
            shellPid = -1;
            if (!safeMode || restartAtMs < 0) enterSafeMode("the shell exited", true);
        }
        drainFails();
    }

    /** Close everything (gui stop / computer shutdown). Returns the shell's pid to kill, or -1. */
    public synchronized int close() {
        closedForGood = true;
        for (UiConnection c : new ArrayList<>(conns.values())) disconnectLocked(c, null);
        int pid = shellPid;
        shellPid = -1;
        restartAtMs = -1;
        return pid;
    }

    public synchronized void setDisplay(UiDisplay d) {
        setDisplayLocked(d);
        if (shell != null) shell.queue(new UiProto.DisplayChanged(d.info()));
        if (safeMode) safeLayout();
        requestTick(0);
    }

    private void setDisplayLocked(UiDisplay d) {
        display = d;
        frame = new int[d.deviceW() * d.deviceH()];
        java.util.Arrays.fill(frame, BG);
        rgba = new byte[frame.length * 4];
        for (int i = 3; i < rgba.length; i += 4) rgba[i] = (byte) 255;
        damage.clear();
        damage.add(displayBox());
    }

    public synchronized UiDisplay display() {
        return display;
    }

    private Box displayBox() {
        return new Box(0, 0, display.deviceW(), display.deviceH());
    }

    // =================================================================== connections

    /**
     * {@code ui_open}: a new connection for program {@code pid}, or null with
     * the host-call error in {@code err[0]} (-2 the shell role is taken, -1 too
     * many connections or the compositor is closed).
     */
    public synchronized UiConnection open(int pid, UiProto.Role role, int[] err) {
        if (closedForGood) {
            err[0] = UiConnection.E_NO_DEVICE;
            return null;
        }
        if (role == UiProto.Role.SHELL && (shell != null || conns.values().stream().anyMatch(UiConnection::isShell))) {
            err[0] = UiConnection.E_BUSY;
            return null;
        }
        long open = conns.values().stream().filter(c -> c.legacy == null).count();
        if (open >= limits.connections()) {
            err[0] = UiConnection.E_INVALID;
            return null;
        }
        UiConnection c = new UiConnection(this, nextConn++, pid, role, null);
        c.opTokens = limits.treeOpsBytesPerSecond();
        c.opRefillMs = host.nowMs();
        conns.put(c.id, c);
        return c;
    }

    synchronized boolean handleAll(UiConnection c, List<UiProto.Message> msgs, int bytes) {
        try {
            for (UiProto.Message m : msgs) {
                if (c.closed) return false;
                handle(c, m);
            }
            return !c.closed;
        } catch (UiError e) {
            disconnectLocked(c, e);
            return false;
        } catch (RuntimeException e) {
            host.log("internal error handling a UI message: " + e);
            disconnectLocked(c, new UiError(UiProto.ErrorCode.INTERNAL, "internal error"));
            return false;
        } finally {
            drainFails();
        }
    }

    synchronized void fail(UiConnection c, UiError e) {
        disconnectLocked(c, e);
        drainFails();
    }

    /** Close {@code c} with {@code e} once the current call finishes (compositor lock held). */
    void failLater(UiConnection c, UiError e) {
        if (c.closed || toFail.contains(c)) return;
        c.closeReason = e;
        toFail.add(c);
    }

    private void drainFails() {
        while (!toFail.isEmpty()) {
            UiConnection c = toFail.remove(0);
            disconnectLocked(c, c.closeReason);
        }
    }

    synchronized void disconnect(UiConnection c, UiError e) {
        disconnectLocked(c, e);
        drainFails();
    }

    private void disconnectLocked(UiConnection c, UiError reason) {
        if (c.closed && !conns.containsKey(c.id)) return;
        for (Surface s : new ArrayList<>(c.surfaces.values())) destroySurface(s);
        c.surfaces.clear();
        c.resources.clear();
        conns.remove(c.id);
        hotkeyClaims.remove(c.id);
        MediaEntry ended = media.remove(c.id);
        if (ended != null) {
            UiProto.MediaInfo none = new UiProto.MediaInfo(ended.info().title(), ended.info().artist(), ended.info().album(), 0, 0,
                    new byte[0], UiProto.MediaState.NONE, 0, 0, List.of());
            if (shell != null && c != shell) shell.queue(new UiProto.MediaSessionChanged(ended.pid(), ended.appId(), none));
            host.mediaSessionChanged(ended.pid(), ended.appId(), none);
        }
        c.markClosed(reason);
        if (reason != null) host.log("closed UI connection of pid " + c.pid + ": " + reason.code.wireName + ": " + reason.getMessage());
        if (c == shell) {
            shell = null;
            chords.clear();
            // Prompts nobody can answer any more: deny them (the app may ask again).
            List<PermissionAnswer> waiting = new ArrayList<>(permissionWaiters.values());
            permissionWaiters.clear();
            for (PermissionAnswer a : waiting) a.answered(false, false);
            long ran = host.nowMs() - shellConnectedMs;
            if (ran >= STABLE_SHELL_MS) backoff = 0;
            enterSafeMode(reason != null ? "the shell broke the protocol" : "the shell closed its connection", true);
        }
    }

    // =================================================================== protocol

    private static final Set<Integer> APP_TO_COMP = Set.of(Opcode.HELLO, Opcode.SURFACE_CREATE, Opcode.SURFACE_DESTROY,
            Opcode.SURFACE_SET_TITLE, Opcode.SURFACE_SET_HINTS, Opcode.SURFACE_RESIZE, Opcode.ACK_CONFIGURE, Opcode.COMMIT,
            Opcode.REQUEST_FRAME, Opcode.TREE_OPS, Opcode.RESOURCE_CREATE, Opcode.RESOURCE_DESTROY, Opcode.HOTKEY_CLAIM,
            Opcode.POINTER_LOCK, Opcode.LAUNCH, Opcode.CLIPBOARD_SET, Opcode.CLIPBOARD_REQUEST, Opcode.NOTIFY,
            Opcode.PICK_FILE, Opcode.OPEN, Opcode.SAVE_STATE, Opcode.BACKGROUND_DONE,
            Opcode.COMMIT_AT, Opcode.QUEUE_FLUSH, Opcode.SURFACE_SET_CONTENT, Opcode.BACKGROUND_MODE, Opcode.MEDIA_SESSION);
    private static final Set<Integer> SHELL_TO_COMP = Set.of(Opcode.PLACE, Opcode.STACK, Opcode.FOCUS_WINDOW,
            Opcode.SET_VISIBLE, Opcode.SET_OPACITY, Opcode.BIND_CHORD, Opcode.CLOSE_WINDOW, Opcode.SET_THEME,
            Opcode.ANSWER_PERMISSION, Opcode.NOTIFICATION_INVOKE, Opcode.MEDIA_CONTROL);

    private void handle(UiConnection c, UiProto.Message m) throws UiError {
        int op = m.opcode();
        if (!APP_TO_COMP.contains(op)) {
            if (SHELL_TO_COMP.contains(op)) {
                if (!c.isShell()) throw UiError.notPermitted(name(m) + " is for the shell only");
            } else {
                throw UiError.badState(name(m) + " goes from the compositor to clients");
            }
        }
        if (!c.hello) {
            if (!(m instanceof UiProto.Hello h)) throw UiError.badState("the first message must be Hello");
            hello(c, h);
            return;
        }
        switch (m) {
            case UiProto.Hello h -> throw UiError.badState("Hello sent twice");
            case UiProto.SurfaceCreate s -> createSurface(c, s);
            case UiProto.SurfaceDestroy s -> destroySurface(surface(c, s.surface()));
            case UiProto.SurfaceSetTitle s -> {
                Surface sf = surface(c, s.surface());
                sf.title = s.title();
                windowChanged(sf);
            }
            case UiProto.SurfaceSetHints s -> {
                Surface sf = surface(c, s.surface());
                sf.minW = s.minW();
                sf.minH = s.minH();
                sf.maxW = s.maxW();
                sf.maxH = s.maxH();
                windowChanged(sf);
            }
            case UiProto.SurfaceResize s -> resize(c, s);
            case UiProto.AckConfigure a -> {
                Surface sf = surface(c, a.surface());
                int[] cfg = sf.configures.get(a.serial());
                if (cfg == null) throw UiError.badState("AckConfigure for serial " + a.serial() + " that wasn't sent");
                sf.configures.keySet().removeIf(k -> k <= a.serial());
                sf.acked = cfg;
            }
            case UiProto.Commit cm -> commit(surface(c, cm.surface()));
            case UiProto.CommitAt cm -> {
                needVersion2(c, m);
                commitAt(c, surface(c, cm.surface()), cm.ptsUs());
            }
            case UiProto.QueueFlush q -> {
                needVersion2(c, m);
                queueFlush(c, surface(c, q.surface()));
            }
            case UiProto.SurfaceSetContent sc -> {
                needVersion2(c, m);
                Surface sf = surface(c, sc.surface());
                if (sc.resource() == 0) {
                    if (!sf.isPixels()) throw UiError.badState("SurfaceSetContent on a tree surface needs a canvas");
                    sf.hint = sc.hint();
                } else {
                    UiResource r = c.resources.get(sc.resource());
                    if (r == null || r.kind != UiProto.ResourceKind.CANVAS) throw UiError.unknown("no canvas " + sc.resource());
                    r.hint = sc.hint();
                }
            }
            case UiProto.RequestFrame rf -> {
                Surface sf = surface(c, rf.surface());
                sf.frameRequested = true;
                // A timed producer runs as far ahead as its queue allows: Frame comes when there is room,
                // not on the refresh tick (DISPLAY_PIPELINE_SPEC.md Part C4).
                if (timedActive(c, sf, DisplayClockNow()) && timedRoom(c, sf)) requestTick(0);
                else requestTick(periodMs());
            }
            case UiProto.TreeOps t -> {
                Surface sf = surface(c, t.surface());
                if (sf.tree == null) throw UiError.badState("TreeOps on a pixel surface");
                long cost = 8L + 16L * t.ops().size();
                for (UiProto.TreeOp o : t.ops()) cost += opBytes(o);
                refill(c);
                c.opTokens -= cost;
                sf.pendingOps.addAll(t.ops());
                sf.pendingOpBytes += cost;
                if (sf.pendingOps.size() > 4L * limits.nodesPerSurface() + 65536) {
                    throw UiError.limit("too many uncommitted tree operations");
                }
            }
            case UiProto.ResourceCreate r -> createResource(c, r);
            case UiProto.ResourceDestroy r -> {
                UiResource res = c.resources.remove(r.id());
                if (res == null) throw UiError.unknown("no resource " + r.id());
                c.resourceBytes -= res.bytes();
                damageResourceUsers(c, r.id());
            }
            case UiProto.HotkeyClaim h -> claimHotkey(c, h);
            case UiProto.PointerLock p -> pointerLock(c, p);
            case UiProto.Launch l -> {
                if (CONSOLE_APP.equals(l.appId())) {
                    host.showConsole();
                    return;
                }
                int pid = host.launch(c.pid, l.appId(), l.args());
                host.log("launch " + l.appId() + " from pid " + c.pid + " -> pid " + pid);
            }
            case UiProto.ClipboardSet cs -> {
                clipboardMime = cs.mime();
                clipboard = cs.data();
                host.clipboardChanged(cs.mime(), cs.data());
            }
            case UiProto.ClipboardRequest cr -> {
                // §12: only right after a paste key or a click in the app's own focused window.
                boolean ok = clipboardReadable(c) && mimeMatches(cr.mime(), clipboardMime) && clipboard.length > 0;
                if (ok) pasteArmed = null; // one paste per key press or click
                c.queue(new UiProto.ClipboardData(ok ? clipboardMime : "", ok ? clipboard : new byte[0]));
            }
            case UiProto.Notify n -> host.checkNotify(c.pid, ok -> {
                synchronized (Compositor.this) {
                    if (ok) post(c, n);
                    else host.log("notification from " + c.appId + " dropped: no notifications permission");
                }
            });
            case UiProto.PickFile p -> pickFile(c, p);
            case UiProto.Open o -> {
                if (o.path().isEmpty() || o.path().length() > 4096) throw UiError.malformed("Open needs a path");
                host.open(c.pid, o.path());
            }
            case UiProto.SaveState s -> {
                // The app manager keeps it under /var/apps/<id>/state (AppManager.onSaveState); non-apps have nowhere to keep it.
                if (!host.saveState(c.pid, s.data())) host.log("SaveState from pid " + c.pid + " dropped: not an app");
            }
            case UiProto.BackgroundDone b -> host.backgroundDone(c.pid);
            case UiProto.BackgroundMode b -> {
                if (!BACKGROUND_MODES.contains(b.mode())) throw UiError.malformed("unknown background mode '" + b.mode() + "'");
                host.backgroundMode(c.pid, b.mode(), b.on());
            }
            case UiProto.MediaSession ms -> mediaSession(c, ms.info());
            case UiProto.MediaControl mc -> mediaAction(mc.pid(), mc.action(), mc.positionMs());
            case UiProto.Place p -> place(p);
            case UiProto.Stack s -> restack(s.windows());
            case UiProto.FocusWindow f -> {
                UiWindow w = windows.get(f.window());
                if (w != null) setFocus(w);
            }
            case UiProto.SetVisible v -> {
                UiWindow w = windows.get(v.window());
                if (w != null && w.visible != v.visible()) {
                    w.visible = v.visible();
                    damageWindow(w);
                    if (!w.visible && w == focused) setFocus(null);
                }
            }
            case UiProto.SetOpacity o -> {
                UiWindow w = windows.get(o.window());
                if (w != null) {
                    w.setAlpha(o.alpha(), o.transition(), host.nowMs());
                    damageWindow(w);
                    requestTick(0);
                }
            }
            case UiProto.BindChord b -> {
                if (b.keys().isEmpty()) chords.remove(b.id());
                else {
                    int[] spec = new int[b.keys().size() + 1];
                    spec[0] = b.mods();
                    for (int i = 0; i < b.keys().size(); i++) spec[i + 1] = b.keys().get(i);
                    chords.put(b.id(), spec);
                }
            }
            case UiProto.CloseWindow cw -> {
                UiWindow w = windows.get(cw.window());
                if (w != null) requestClose(w);
            }
            case UiProto.SetTheme st -> {
                theme = st.theme();
                for (UiConnection o : conns.values()) if (o.hello) o.queue(new UiProto.ThemeChanged(theme));
            }
            case UiProto.AnswerPermission a -> {
                PermissionAnswer w = permissionWaiters.remove(a.req());
                if (w != null) {
                    try {
                        w.answered(a.allow(), a.remember());
                    } catch (RuntimeException e) {
                        host.log("permission answer handler failed: " + e);
                    }
                }
            }
            case UiProto.NotificationInvoke ni -> invokeNotification(ni);
            default -> throw UiError.badState(name(m) + " is not expected here");
        }
    }

    // =================================================================== background modes and media (§11, §12)

    /** The modes BackgroundMode may name (app.toml's, AppManifest.BACKGROUND_MODES). */
    static final Set<String> BACKGROUND_MODES = Set.of("audio", "call", "radio", "transfer", "navigation");
    /** What MediaAction / MediaControl may ask for. */
    static final Set<String> MEDIA_ACTIONS = Set.of("play", "pause", "next", "previous", "seek");

    /** One app's media session (the Now Playing card). */
    public record MediaEntry(int pid, String appId, UiProto.MediaInfo info, long atMs) {
        public boolean playing() {
            return info.state() == UiProto.MediaState.PLAYING || info.state() == UiProto.MediaState.BUFFERING;
        }
    }

    /** Media sessions by connection id. */
    private final Map<Integer, MediaEntry> media = new LinkedHashMap<>();
    /** Background modes by pid (app id, modes), for a shell that connects later. */
    private final Map<Integer, UiProto.AppModes> appModes = new LinkedHashMap<>();

    private void mediaSession(UiConnection c, UiProto.MediaInfo info) throws UiError {
        if ((long) info.artW() * info.artH() * 4 != info.artRgba().length) {
            throw UiError.malformed("MediaSession art is " + info.artRgba().length + " bytes, not " + info.artW() + " x " + info.artH() + " x 4");
        }
        for (String a : info.actions()) if (!MEDIA_ACTIONS.contains(a)) throw UiError.malformed("unknown media action '" + a + "'");
        String appId = c.appId == null ? "" : c.appId;
        if (info.state() == UiProto.MediaState.NONE) media.remove(c.id);
        else media.put(c.id, new MediaEntry(c.pid, appId, info, host.nowMs()));
        if (shell != null) shell.queue(new UiProto.MediaSessionChanged(c.pid, appId, info));
        host.mediaSessionChanged(c.pid, appId, info);
    }

    /** The media sessions now, oldest first. */
    public synchronized List<MediaEntry> mediaSessions() {
        return new ArrayList<>(media.values());
    }

    /**
     * Send {@code MediaAction{action}} to program {@code pid}'s media session (the shell's
     * Now Playing card, AirPods). False if it has none.
     */
    public synchronized boolean mediaAction(int pid, String action, int positionMs) {
        if (!MEDIA_ACTIONS.contains(action)) return false;
        boolean sent = false;
        for (Map.Entry<Integer, MediaEntry> e : media.entrySet()) {
            UiConnection c = conns.get(e.getKey());
            if (e.getValue().pid() == pid && c != null && !c.closed) {
                c.queue(new UiProto.MediaAction(action, Math.max(0, positionMs)));
                sent = true;
            }
        }
        return sent;
    }

    /**
     * The session a play/pause button with no screen (AirPods) means: the one playing, else
     * the one changed last. Null if there is none.
     */
    public synchronized MediaEntry activeMedia() {
        MediaEntry best = null;
        for (MediaEntry m : media.values()) {
            if (best == null || m.playing() && !best.playing() || m.playing() == best.playing() && m.atMs() >= best.atMs()) best = m;
        }
        return best;
    }

    /** An app's background modes changed: tell the shell ({@code AppModes}; the desktop tray, the phone's pills). */
    public synchronized void appModesChanged(int pid, String appId, java.util.Collection<String> modes) {
        UiProto.AppModes m = new UiProto.AppModes(pid, appId, List.copyOf(new java.util.TreeSet<>(modes)));
        if (modes.isEmpty()) appModes.remove(pid);
        else appModes.put(pid, m);
        if (shell != null) shell.queue(m);
    }

    // =================================================================== services (§10, §12)

    /** A notification from the system about app {@code appId} (no app connection posts it). */
    public synchronized void postSystem(String appId, String tag, String title, String body) {
        int nid = nextNid++;
        notifications.put(nid, new Object[]{null, tag, appId});
        posted.add(new PostedNotification(nid, appId, tag, title, body, host.nowMs()));
        if (posted.size() > 64) posted.remove(0);
        if (shell != null) shell.queue(new UiProto.NotificationPosted(nid, appId, tag, title, body, "", List.of()));
    }

    /** Post a notification that passed its permission check. */
    private void post(UiConnection c, UiProto.Notify n) {
        int nid = nextNid++;
        notifications.put(nid, new Object[]{c, n.tag(), c.appId});
        posted.add(new PostedNotification(nid, c.appId == null ? "" : c.appId, n.tag(), n.title(), n.body(), host.nowMs()));
        if (posted.size() > 64) posted.remove(0);
        if (shell != null) {
            shell.queue(new UiProto.NotificationPosted(nid, c.appId == null ? "" : c.appId, n.tag(), n.title(), n.body(), n.icon(), n.actions()));
        }
    }

    /**
     * {@code OpenFile{path}} for the app whose root process is {@code pid} (open
     * with; its grant is in place): sent now to its connections that said Hello,
     * or right after {@code Welcome} when it connects (a freshly launched app).
     */
    public synchronized void deliverOpenFile(int pid, String path) {
        boolean sent = false;
        for (UiConnection c : conns.values()) {
            if (c.pid == pid && c.hello && !c.closed && !c.isShell()) {
                c.queue(new UiProto.OpenFile(path));
                sent = true;
            }
        }
        if (!sent) pendingOpenFiles.computeIfAbsent(pid, k -> new ArrayList<>()).add(path);
    }

    /** May {@code c} read the clipboard now? Its window has focus and it just got a paste key or a click. */
    private boolean clipboardReadable(UiConnection c) {
        return focused != null && focused.surface.conn == c && pasteArmed == c
                && host.nowMs() - pasteArmedAt <= PASTE_WINDOW_MS;
    }

    /** {@code text/plain} matches {@code text/plain;charset=utf-8}: the parameters don't count. */
    static boolean mimeMatches(String want, String have) {
        if (have.isEmpty()) return false;
        if (want.isEmpty() || want.equals(have)) return true;
        return base(want).equalsIgnoreCase(base(have));
    }

    private static String base(String mime) {
        int i = mime.indexOf(';');
        return (i < 0 ? mime : mime.substring(0, i)).trim();
    }

    private static boolean isPasteKey(int hid, int mods) {
        int m = mods & UiProto.Mods.MASK;
        return (hid == HID_V && (m & UiProto.Mods.CTRL) != 0) || (hid == HID_INSERT && m == UiProto.Mods.SHIFT);
    }

    private void invokeNotification(UiProto.NotificationInvoke ni) {
        Object[] n = notifications.get(ni.nid());
        if (n == null) return;
        UiConnection app = (UiConnection) n[0];
        UiProto.NotificationAction action = new UiProto.NotificationAction((String) n[1], ni.action());
        if (app == null) {
            // A system notice about an app (the background limit, telephonyd's missed call or
            // new text): tapping it opens that app, which gets NotificationAction{tag, action}
            // so it can show the right screen (the thread, the call log).
            String appId = (String) n[2];
            if (appId == null || appId.isEmpty()) return;
            String tag = (String) n[1];
            if (tag != null && !tag.isEmpty()) {
                for (UiConnection c : conns.values()) {
                    if (appId.equals(c.appId) && c.hello && !c.closed && !c.isShell()) {
                        c.queue(action);
                        host.raise(appId);
                        return;
                    }
                }
                pendingActions.computeIfAbsent(appId, k -> new ArrayList<>()).add(action);
            }
            host.launch(0, appId, List.of());
            return;
        }
        if (!app.closed) {
            app.queue(action);
            return;
        }
        // The app has quit: start it again and hand it the action once it connects (§12).
        String appId = (String) n[2];
        if (appId == null || appId.isEmpty()) return;
        pendingActions.computeIfAbsent(appId, k -> new ArrayList<>()).add(action);
        int pid = host.launch(0, appId, List.of());
        host.log("notification action relaunches " + appId + " -> pid " + pid);
    }

    /**
     * {@code PickFile} from an app (§12): the system picker ({@link SystemDialogs})
     * draws the choice unless another picker was installed with
     * {@link #setFilePicker}. The answer comes back as {@code FilePicked{req, path}}
     * ("" when cancelled), and a chosen file becomes a one-file read-write grant
     * for the app ({@link CompositorHost#filePicked}).
     */
    private void pickFile(UiConnection c, UiProto.PickFile p) {
        java.util.function.Consumer<String> answer = picked -> {
            String path = picked == null ? "" : picked;
            synchronized (Compositor.this) {
                if (!c.closed) c.queue(new UiProto.FilePicked(p.req(), path));
            }
            if (!path.isEmpty()) host.filePicked(c.pid, path, true);
        };
        FilePicker picker = filePicker;
        if (picker != null) picker.pick(c.appId, p, answer);
        else dialogs.pick(c.pid, c.appId, windowOf(c), p, answer);
    }

    /** Hand {@code PickFile} to a different picker (null = the system picker). */
    public synchronized void setFilePicker(FilePicker picker) {
        filePicker = picker;
    }

    /**
     * Ask the player, through the shell's dialog, whether {@code appId} may use
     * {@code permission} (§10 {@code PermissionPrompt}). The answer arrives once on
     * {@code answer}; without a shell (safe mode) the answer is "deny, don't remember".
     * Returns the request number.
     */
    public synchronized int requestPermission(String appId, String permission, PermissionAnswer answer) {
        int req = nextPermissionReq++;
        if (shell == null || shell.closed) {
            answer.answered(false, false);
            return req;
        }
        permissionWaiters.put(req, answer);
        shell.queue(new UiProto.PermissionPrompt(req, appId == null ? "" : appId, permission));
        return req;
    }

    private static String name(UiProto.Message m) {
        return m.getClass().getSimpleName();
    }

    private static long opBytes(UiProto.TreeOp o) {
        return switch (o) {
            case UiProto.TreeOp.Create c -> 14 + propBytes(c.props());
            case UiProto.TreeOp.Set s -> 6 + propBytes(s.props());
            case UiProto.TreeOp.Move m -> 13;
            case UiProto.TreeOp.Remove r -> 5;
        };
    }

    private static long propBytes(List<UiProto.Prop> ps) {
        long n = 0;
        for (UiProto.Prop p : ps) {
            n += 5;
            if (p instanceof UiProto.Prop.Text t) n += t.value().length();
            if (p instanceof UiProto.Prop.Points pts) n += 8L * pts.value().size();
        }
        return n;
    }

    private void hello(UiConnection c, UiProto.Hello h) throws UiError {
        // Version 1 apps keep working: their messages mean what they did, and they never get v2 events.
        if (h.version() < 1 || h.version() > UiProto.VERSION) {
            throw new UiError(UiProto.ErrorCode.VERSION, "protocol version " + h.version() + " not supported (have 1-" + UiProto.VERSION + ")");
        }
        if (h.role() != c.role) throw UiError.badState("Hello role differs from ui_open");
        c.hello = true;
        c.version = h.version();
        c.appId = host.appId(c.pid);
        // An app's saved state comes back right after Welcome (§11 Restore; P4/P5 wiring).
        byte[] restore = c.isShell() ? null : host.restoreFor(c.pid);
        c.queue(new UiProto.Welcome(c.version, c.appId == null ? "" : c.appId, display.info(), theme,
                UiFonts.METRICS_VERSION, List.of(), restore != null));
        if (restore != null) c.queue(new UiProto.Restore(restore));
        List<UiProto.NotificationAction> waiting = c.isShell() ? null : pendingActions.remove(c.appId);
        if (waiting != null) for (UiProto.NotificationAction a : waiting) c.queue(a);
        List<String> files = c.isShell() ? null : pendingOpenFiles.remove(c.pid);
        if (files != null) for (String f : files) c.queue(new UiProto.OpenFile(f));
        // Version 2: the queue limits for CommitAt, after what has to come right after Welcome.
        if (c.version >= 2) {
            c.queue(new UiProto.QueueLimits(com.example.evanscomputermod.EcmConfig.displayQueueFrames(),
                    com.example.evanscomputermod.EcmConfig.displayQueueMs()));
        }
        if (c.isShell()) {
            shell = c;
            shellConnectedMs = host.nowMs();
            if (c.pid != shellPid && shellPid > 0) host.log("shell connected from pid " + c.pid + " (started pid " + shellPid + ")");
            for (UiWindow w : stack) shell.queue(opened(w));
            for (MediaEntry m : media.values()) shell.queue(new UiProto.MediaSessionChanged(m.pid(), m.appId(), m.info()));
            for (UiProto.AppModes m : appModes.values()) shell.queue(m);
            leaveSafeMode();
        }
    }

    private UiProto.WindowOpened opened(UiWindow w) {
        Surface s = w.surface;
        return new UiProto.WindowOpened(w.id, s.conn.pid, s.conn.appId == null ? "" : s.conn.appId, s.title, "",
                s.minW, s.minH, s.maxW, s.maxH, s.parent != null && s.parent.window != null ? s.parent.window.id
                        : dialogs.parentWindowOf(s.conn.pid));
    }

    private Surface surface(UiConnection c, int id) throws UiError {
        Surface s = c.surfaces.get(id);
        if (s == null) throw UiError.unknown("no surface " + id);
        return s;
    }

    private void createSurface(UiConnection c, UiProto.SurfaceCreate s) throws UiError {
        if (c.surfaces.containsKey(s.surface())) throw UiError.badState("surface " + s.surface() + " already exists");
        if (c.surfaces.size() >= limits.surfacesPerApp()) throw UiError.limit("more than " + limits.surfacesPerApp() + " surfaces");
        Surface parent = null;
        switch (s.role()) {
            case LAYER -> {
                if (!c.isShell()) throw UiError.notPermitted("only the shell makes layer surfaces");
                if (s.layer() == UiProto.Layer.NONE) throw UiError.malformed("a layer surface needs a layer");
            }
            case POPUP -> {
                parent = surface(c, s.parent());
                if (s.layer() != UiProto.Layer.NONE) throw UiError.malformed("only layer surfaces have a layer");
            }
            case WINDOW -> {
                if (s.layer() != UiProto.Layer.NONE) throw UiError.malformed("only layer surfaces have a layer");
            }
        }
        if (s.kind() == UiProto.SurfaceKind.PIXELS) {
            if (s.format() == UiProto.PixelFormat.NONE) throw UiError.malformed("a pixel surface needs a format");
            checkPixelSize(s.w(), s.h(), s.scale());
            long bytes = (long) s.w() * s.h() * (4 + TerminalDisplay.bytesPerPixel(s.format().code));
            charge(c, bytes);
        }
        Surface sf = new Surface(c, s, parent);
        c.surfaces.put(sf.id, sf);
        switch (s.role()) {
            case WINDOW -> {
                UiWindow w = new UiWindow(nextWindow++, sf);
                sf.window = w;
                windows.put(w.id, w);
                stack.add(w);
                // The shell hears about its own windows too (its window frames), so it can place and stack them.
                if (shell != null) shell.queue(opened(w));
                if (safeMode || shell == null && focused == null) {
                    setFocus(w);
                }
                if (safeMode) safeLayout();
            }
            case LAYER -> {
                layers.add(sf);
                int w = s.w() > 0 ? s.w() : display.logicalW();
                int h = s.h() > 0 ? s.h() : display.logicalH();
                sf.sendConfigure(sf.isPixels() ? sf.naturalW() : w, sf.isPixels() ? sf.naturalH() : h, 0);
            }
            case POPUP -> {
                popups.add(sf);
                sf.sendConfigure(sf.isPixels() ? sf.naturalW() : s.w(), sf.isPixels() ? sf.naturalH() : s.h(), 0);
            }
        }
    }

    private void checkPixelSize(int w, int h, int scale) throws UiError {
        if (w < 1 || h < 1) throw UiError.malformed("a pixel surface needs a size");
        if (w > display.deviceW() || h > display.deviceH()) {
            throw UiError.limit("pixel surface " + w + "x" + h + " is larger than the display (" + display.deviceW() + "x" + display.deviceH() + ")");
        }
    }

    private void charge(UiConnection c, long bytes) throws UiError {
        if (c.resourceBytes + bytes > limits.resourceBytesPerApp()) {
            throw UiError.limit("more than " + limits.resourceBytesPerApp() / 1024 + " KiB of resources");
        }
        c.resourceBytes += bytes;
    }

    private void destroySurface(Surface s) {
        UiConnection c = s.conn;
        damageSurface(s);
        // Popups of this surface go with it.
        for (Surface p : new ArrayList<>(popups)) if (p.parent == s) destroySurface(p);
        c.surfaces.remove(s.id);
        c.nodes -= s.tree != null ? s.tree.size() : 0;
        c.resourceBytes -= s.bytes();
        layers.remove(s);
        popups.remove(s);
        if (hover == s) hover = null;
        if (grab == s) grab = null;
        if (pointerLocked == s) pointerLocked = null;
        UiWindow w = s.window;
        if (w != null) {
            windows.remove(w.id);
            stack.remove(w);
            if (shell != null && !(c == shell && c.closed)) shell.queue(new UiProto.WindowClosed(w.id));
            if (focused == w) {
                focused = null;
                host.focusChanged(c.pid, -1);
                UiWindow next = topmostShown();
                if (next != null && (safeMode || shell == null)) setFocus(next);
            }
            if (safeMode) safeLayout();
        }
    }

    private void resize(UiConnection c, UiProto.SurfaceResize r) throws UiError {
        Surface s = surface(c, r.surface());
        if (!s.isPixels()) throw UiError.badState("SurfaceResize on a tree surface");
        if (r.format() == UiProto.PixelFormat.NONE) throw UiError.malformed("a pixel surface needs a format");
        checkPixelSize(r.w(), r.h(), r.scale());
        long old = s.bytes();
        long now = (long) r.w() * r.h() * (4 + TerminalDisplay.bytesPerPixel(r.format().code));
        c.resourceBytes -= old;
        charge(c, now);
        s.resizePending = true;
        s.newBw = r.w();
        s.newBh = r.h();
        s.newFormat = r.format().code;
        s.newScale = r.scale();
        s.pendingRaw = new byte[r.w() * r.h() * TerminalDisplay.bytesPerPixel(r.format().code)];
        s.pendingPixels.clear();
    }

    private void createResource(UiConnection c, UiProto.ResourceCreate r) throws UiError {
        if (c.resources.containsKey(r.id())) throw UiError.badState("resource " + r.id() + " already exists");
        int fmt;
        if (r.kind() == UiProto.ResourceKind.CANVAS) {
            if (r.format() == UiProto.PixelFormat.NONE) throw UiError.malformed("a canvas needs a format");
            fmt = r.format().code;
            checkPixelSize(r.w(), r.h(), 1);
        } else {
            fmt = TerminalDisplay.PIXEL_FORMAT_RGBA8888;
            if ((long) r.w() * r.h() > 4096L * 4096L) throw UiError.limit("image too large");
        }
        UiResource res = new UiResource(r.id(), r.kind(), r.w(), r.h(), fmt);
        charge(c, res.bytes());
        c.resources.put(res.id, res);
    }

    private void refill(UiConnection c) {
        long now = host.nowMs();
        long dt = now - c.opRefillMs;
        if (dt > 0) {
            double rate = limits.treeOpsBytesPerSecond();
            c.opTokens = Math.min(rate, c.opTokens + rate * dt / 1000.0);
            c.opRefillMs = now;
        }
    }

    /** Apply everything pending on {@code s} (§5 "Changes are atomic"). */
    private void commit(Surface s) throws UiError {
        UiConnection c = s.conn;
        if (s.tree != null && !s.pendingOps.isEmpty()) {
            refill(c);
            if (c.opTokens < 0) {
                // Over the tree-ops rate: merge this commit into a later one (§6 limits).
                s.commitDeferred = true;
                requestTick(periodMs());
                return;
            }
        }
        s.commitDeferred = false;
        applyCommit(s);
    }

    private void applyCommit(Surface s) throws UiError {
        UiConnection c = s.conn;
        Box.Region d = new Box.Region();
        if (s.tree != null) {
            for (UiProto.TreeOp op : s.pendingOps) {
                int budget = limits.nodesPerApp() - c.nodes;
                c.nodes += s.tree.apply(op, limits.nodesPerSurface(), budget, d);
                if (exporter != null) exporter.applied(s, op);
            }
            s.pendingOps.clear();
            s.pendingOpBytes = 0;
        } else {
            if (s.resizePending) {
                byte[] raw = s.pendingRaw;
                s.setBuffer(s.newBw, s.newBh, s.newFormat, s.newScale);
                s.pendingRaw = raw;
                Px.convert(raw, s.format, s.palette, s.bw, s.bh, 0, 0, s.bw, s.bh, s.argb);
                s.resizePending = false;
                s.damageAll = true;
            } else {
                for (Box b : s.pendingPixels.boxes()) {
                    Px.convert(s.pendingRaw, s.format, s.palette, s.bw, s.bh, b.x0(), b.y0(), b.w(), b.h(), s.argb);
                    d.add(b.scale(s.pscale));
                }
            }
            if (!s.pendingPixels.isEmpty()) s.pixelVersion++;
            s.pendingPixels.clear();
        }
        for (UiResource r : c.resources.values()) {
            if (r.kind == UiProto.ResourceKind.CANVAS && r.commitCanvas()) damageResourceUsers(c, r.id);
        }
        if (s.acked != null) {
            int[] cfg = s.acked;
            s.acked = null;
            boolean sizeChanged = !s.isPixels() && (cfg[0] != s.contentW || cfg[1] != s.contentH);
            if (sizeChanged) damageSurface(s);
            if (!s.isPixels()) {
                s.contentW = cfg[0];
                s.contentH = cfg[1];
            }
            s.states = cfg[2];
            if (!s.mapped || sizeChanged) s.damageAll = true;
            s.mapped = true;
        }
        if (s.mapped) {
            if (s.damageAll) {
                s.damageAll = false;
                damageSurface(s);
            } else {
                for (Box b : d.boxes()) damageSurfaceLu(s, b);
            }
        }
        requestTick(0);
    }

    /**
     * A client scrolled a scroll container it draws itself (Part F): the app
     * that owns scene surface {@code sceneSurface} gets {@code Scrolled} (if
     * it speaks version 2) and confirms with its next commit.
     */
    public synchronized void scrolled(int sceneSurface, int node, int x, int y) {
        SceneExporter ex = exporter;
        Surface s = ex == null ? null : ex.surfaceOf(sceneSurface);
        if (s == null || s.conn.closed || s.conn.version < 2 || s.tree == null) return;
        NodeTree.Node n = s.tree.get(node);
        if (n == null || n.scroll == 0) return;
        int cx = Math.max(0, Math.min(Math.max(0, n.contentW - n.w), x));
        int cy = Math.max(0, Math.min(Math.max(0, n.contentH - n.h), y));
        s.conn.queue(new UiProto.Scrolled(s.id, node, cx, cy));
        drainFails();
    }

    // =================================================================== timed commits (protocol v2)

    private static long DisplayClockNow() {
        return com.example.evanscomputermod.computer.pipeline.DisplayClock.nowUs();
    }

    private void needVersion2(UiConnection c, UiProto.Message m) throws UiError {
        if (c.version < 2) throw UiError.badState(name(m) + " needs protocol version 2");
    }

    /** A timed stream runs on this surface (or on the connection's canvases) when frames were queued or shown in the last second. */
    private boolean timedActive(UiConnection c, Surface s, long nowUs) {
        if (s.timed != null && s.timed.active(nowUs, 1_000_000)) return true;
        for (UiResource r : c.resources.values()) if (r.timed != null && r.timed.active(nowUs, 1_000_000)) return true;
        return false;
    }

    /** Is there room for one more frame in every queue the surface's next CommitAt fills? */
    private boolean timedRoom(UiConnection c, Surface s) {
        int max = Math.max(1, com.example.evanscomputermod.EcmConfig.displayQueueFrames());
        if (s.timed != null && s.timed.size() >= max) return false;
        for (UiResource r : c.resources.values()) if (r.timed != null && r.timed.size() >= max) return false;
        return true;
    }

    /**
     * {@code CommitAt}: the pixel changes of this commit (the surface's own if
     * it is a pixel surface, and every canvas of the connection with new rows)
     * are queued for {@code ptsMono} (the app's monotonic clock), not shown.
     * Tree changes in it are an error: layout stays immediate.
     */
    private void commitAt(UiConnection c, Surface s, long ptsMono) throws UiError {
        if (s.tree != null && !s.pendingOps.isEmpty()) throw UiError.badState("tree changes in a CommitAt (send them with Commit)");
        if (s.isPixels() && s.resizePending) throw UiError.badState("a resize in a CommitAt (send it with Commit)");
        long now = DisplayClockNow();
        long pts = com.example.evanscomputermod.computer.pipeline.DisplayClock.fromMonotonicUs(ptsMono,
                com.example.evanscomputermod.computer.wasi.WasiFunctions.monotonicNowUs());
        long maxAhead = Math.max(1, com.example.evanscomputermod.EcmConfig.displayQueueMs()) * 1000L;
        if (pts > now + maxAhead) pts = now + maxAhead;
        int max = Math.max(1, com.example.evanscomputermod.EcmConfig.displayQueueFrames());
        boolean any = false;
        if (s.isPixels() && !s.pendingPixels.isEmpty()) {
            TimedQueue q = s.timed();
            int[] base = q.latest() != null ? q.latest() : s.argb;
            int[] frame = base.clone();
            for (Box b : s.pendingPixels.boxes()) {
                Px.convert(s.pendingRaw, s.format, s.palette, s.bw, s.bh, b.x0(), b.y0(), b.w(), b.h(), frame);
            }
            s.pendingPixels.clear();
            if (q.size() >= max) showTimed(c, s, null, q.takeOldest(now));
            TimedQueue.Frame f = q.add(pts, frame, now);
            if (exporter != null) exporter.timedFrame(s, null, null, f, s.hint);
            any = true;
        }
        for (UiResource r : c.resources.values()) {
            if (r.kind != UiProto.ResourceKind.CANVAS || r.pendingRaw == null || r.pendingDamage.isEmpty()) continue;
            TimedQueue q = r.timed();
            int[] frame = (q.latest() != null ? q.latest() : r.argb).clone();
            for (Box b : r.pendingDamage.boxes()) {
                Px.convert(r.pendingRaw, r.format, null, r.w, r.h, b.x0(), b.y0(), b.w(), b.h(), frame);
            }
            r.pendingDamage.clear();
            if (q.size() >= max) showTimed(c, null, r, q.takeOldest(now));
            TimedQueue.Frame f = q.add(pts, frame, now);
            if (exporter != null) exporter.timedFrame(null, c, r, f, r.hint);
            any = true;
        }
        if (s.acked != null || !s.mapped) commit(s); // a configure answered with CommitAt still maps the surface
        if (any) {
            long delayNs = com.example.evanscomputermod.computer.pipeline.DisplayClock.toNanoTime(pts) - System.nanoTime();
            com.example.evanscomputermod.computer.pipeline.DisplayPump.schedule(this::applyTimed, delayNs);
            host.timedQueued();
        }
    }

    /** Make {@code f} what the surface or canvas shows (compositor lock held). */
    private void showTimed(UiConnection c, Surface s, UiResource r, TimedQueue.Frame f) {
        if (f == null) return;
        if (s != null) {
            System.arraycopy(f.argb(), 0, s.argb, 0, Math.min(f.argb().length, s.argb.length));
            s.pixelVersion++;
            if (s.mapped) damageSurface(s);
            if (exporter != null) exporter.timedShown(s, null, null, f.seq());
        } else {
            System.arraycopy(f.argb(), 0, r.argb, 0, Math.min(f.argb().length, r.argb.length));
            r.version++;
            damageResourceUsers(c, r.id);
            if (exporter != null) exporter.timedShown(null, c, r, f.seq());
        }
    }

    /**
     * Pump timer: every queue shows the newest frame that is due (so this
     * server's frame, the Java rasterizer, tile viewers and screenshots see
     * what is on screen now), then the composite and the Frame events follow
     * in a tick.
     */
    public synchronized void applyTimed() {
        if (closedForGood) return;
        long now = DisplayClockNow();
        boolean shown = false;
        for (UiConnection c : conns.values()) {
            for (Surface s : c.surfaces.values()) {
                if (s.timed == null || s.timed.isEmpty()) continue;
                List<TimedQueue.Frame> due = s.timed.takeDue(now);
                if (!due.isEmpty()) {
                    showTimed(c, s, null, due.get(due.size() - 1));
                    shown = true;
                }
            }
            for (UiResource r : c.resources.values()) {
                if (r.timed == null || r.timed.isEmpty()) continue;
                List<TimedQueue.Frame> due = r.timed.takeDue(now);
                if (!due.isEmpty()) {
                    showTimed(c, null, r, due.get(due.size() - 1));
                    shown = true;
                }
            }
        }
        if (shown) requestTick(0);
    }

    /** {@code QueueFlush}: drop the surface's queued frames and the connection's canvases'. */
    private void queueFlush(UiConnection c, Surface s) {
        if (s.timed != null && s.timed.flush() > 0 && exporter != null) exporter.timedFlushed(s, null, null, s.timed.appliedSeq);
        for (UiResource r : c.resources.values()) {
            if (r.timed != null && r.timed.flush() > 0 && exporter != null) {
                exporter.timedFlushed(null, c, r, r.timed.appliedSeq);
            }
        }
        host.timedQueued();
        requestTick(0);
    }

    /** Timed-frame counters of a connection's canvases and surfaces: {queued, shown, dropped, flushed} (diagnostics). */
    public synchronized long[] timedCounters() {
        long[] t = new long[4];
        for (UiConnection c : conns.values()) {
            for (Surface s : c.surfaces.values()) add(t, s.timed);
            for (UiResource r : c.resources.values()) add(t, r.timed);
        }
        return t;
    }

    private static void add(long[] t, TimedQueue q) {
        if (q == null) return;
        t[0] += q.queued;
        t[1] += q.shown;
        t[2] += q.dropped;
        t[3] += q.flushed;
    }

    private void damageResourceUsers(UiConnection c, int resource) {
        for (Surface s : c.surfaces.values()) {
            if (s.tree == null || !s.mapped) continue;
            for (NodeTree.Node n : s.tree.usersOf(resource)) damageSurfaceLu(s, NodeTree.absBox(n));
        }
    }

    // =================================================================== bulk paths

    synchronized boolean putPixels(UiConnection c, int target, int len, int[] rects, UiConnection.PixelSource src) {
        try {
            if (!c.hello) throw UiError.badState("ui_put_pixels before Hello");
            Surface s = c.surfaces.get(target);
            byte[] raw;
            int bw, bh, fmt;
            Box.Region pending;
            if (s != null && s.isPixels()) {
                raw = s.pendingRaw;
                bw = s.resizePending ? s.newBw : s.bw;
                bh = s.resizePending ? s.newBh : s.bh;
                fmt = s.resizePending ? s.newFormat : s.format;
                pending = s.pendingPixels;
            } else {
                UiResource r = c.resources.get(target);
                if (r == null || r.kind != UiProto.ResourceKind.CANVAS) throw UiError.unknown("no pixel surface or canvas " + target);
                if (r.pendingRaw == null) r.pendingRaw = new byte[r.w * r.h * TerminalDisplay.bytesPerPixel(r.format)];
                raw = r.pendingRaw;
                bw = r.w;
                bh = r.h;
                fmt = r.format;
                pending = r.pendingDamage;
            }
            int bpp = TerminalDisplay.bytesPerPixel(fmt);
            if (len != bw * bh * bpp) {
                throw UiError.malformed("ui_put_pixels: " + len + " bytes for a " + bw + "x" + bh + " target needs " + bw * bh * bpp);
            }
            List<Box> boxes = new ArrayList<>();
            if (rects.length == 0) boxes.add(new Box(0, 0, bw, bh));
            for (int i = 0; i + 3 < rects.length; i += 4) {
                Box b = Box.xywh(rects[i], rects[i + 1], rects[i + 2], rects[i + 3]);
                if (rects[i] < 0 || rects[i + 1] < 0 || rects[i + 2] < 0 || rects[i + 3] < 0
                        || b.x1() > bw || b.y1() > bh) {
                    throw UiError.malformed("damage rectangle outside the target");
                }
                if (!b.isEmpty()) boxes.add(b);
            }
            for (Box b : boxes) {
                if (b.x0() == 0 && b.w() == bw) {
                    int off = b.y0() * bw * bpp;
                    src.read(off, raw, off, b.h() * bw * bpp);
                } else {
                    for (int y = b.y0(); y < b.y1(); y++) {
                        int off = (y * bw + b.x0()) * bpp;
                        src.read(off, raw, off, b.w() * bpp);
                    }
                }
                pending.add(b);
            }
            return true;
        } catch (UiError e) {
            disconnectLocked(c, e);
            drainFails();
            return false;
        }
    }

    synchronized boolean upload(UiConnection c, int id, int kind, byte[] data) {
        try {
            if (!c.hello) throw UiError.badState("ui_upload before Hello");
            UiResource r = c.resources.get(id);
            if (r == null || r.kind != UiProto.ResourceKind.IMAGE) throw UiError.unknown("no image resource " + id);
            int[] px = new int[r.w * r.h];
            byte[] canonical;
            if (kind == 1) {
                if (data.length != r.w * r.h * 4) throw UiError.malformed("raw RGBA upload must be w*h*4 bytes");
                canonical = data;
            } else if (kind == 0) {
                canonical = decodePng(data, r.w, r.h);
            } else {
                throw UiError.malformed("unknown upload kind " + kind);
            }
            for (int i = 0; i < px.length; i++) {
                px[i] = Px.premul(canonical[i * 4] & 0xFF, canonical[i * 4 + 1] & 0xFF, canonical[i * 4 + 2] & 0xFF, canonical[i * 4 + 3] & 0xFF);
            }
            r.argb = px;
            r.hash = UiResource.fnv64(canonical);
            damageResourceUsers(c, id);
            requestTick(0);
            return true;
        } catch (UiError e) {
            disconnectLocked(c, e);
            drainFails();
            return false;
        }
    }

    private static byte[] decodePng(byte[] data, int w, int h) throws UiError {
        if (data.length < 24 || (data[0] & 0xFF) != 0x89 || data[1] != 'P' || data[2] != 'N' || data[3] != 'G') {
            throw UiError.malformed("not a PNG");
        }
        int pw = java.nio.ByteBuffer.wrap(data, 16, 4).getInt(), ph = java.nio.ByteBuffer.wrap(data, 20, 4).getInt();
        if (pw != w || ph != h) throw UiError.malformed("PNG is " + pw + "x" + ph + ", the resource " + w + "x" + h);
        try {
            java.awt.image.BufferedImage img = javax.imageio.ImageIO.read(new java.io.ByteArrayInputStream(data));
            if (img == null || img.getWidth() != w || img.getHeight() != h) throw UiError.malformed("unreadable PNG");
            byte[] out = new byte[w * h * 4];
            for (int y = 0; y < h; y++) {
                for (int x = 0; x < w; x++) {
                    int argb = img.getRGB(x, y);
                    int i = (y * w + x) * 4;
                    out[i] = (byte) (argb >>> 16);
                    out[i + 1] = (byte) (argb >>> 8);
                    out[i + 2] = (byte) argb;
                    out[i + 3] = (byte) (argb >>> 24);
                }
            }
            return out;
        } catch (java.io.IOException | RuntimeException e) {
            throw UiError.malformed("unreadable PNG");
        }
    }

    // =================================================================== legacy windows

    /**
     * A pixel window for a legacy program ({@code gfx_init2} while the GUI runs,
     * §1 / ROADMAP P2 "Legacy compat"). The returned connection is internal: no
     * messages, its input goes to {@code sink}.
     */
    public synchronized UiConnection openLegacy(int pid, String title, int w, int h, int format, UiConnection.LegacySink sink) {
        UiConnection c = new UiConnection(this, nextConn++, pid, UiProto.Role.APP, sink);
        c.hello = true;
        c.appId = host.appId(pid);
        conns.put(c.id, c);
        int bw = Math.max(1, Math.min(w, display.deviceW())), bh = Math.max(1, Math.min(h, display.deviceH()));
        UiProto.PixelFormat pf = format == TerminalDisplay.PIXEL_FORMAT_RGBA8888 ? UiProto.PixelFormat.RGBA8888
                : format == TerminalDisplay.PIXEL_FORMAT_RGB565 ? UiProto.PixelFormat.RGB565 : UiProto.PixelFormat.INDEXED8;
        Surface sf = new Surface(c, new UiProto.SurfaceCreate(1, UiProto.SurfaceKind.PIXELS, UiProto.SurfaceRole.WINDOW,
                UiProto.Layer.NONE, 0, 0, 0, bw, bh, pf, 1, title == null ? "" : title), null);
        c.surfaces.put(1, sf);
        UiWindow win = new UiWindow(nextWindow++, sf);
        sf.window = win;
        windows.put(win.id, win);
        stack.add(win);
        if (shell != null) shell.queue(opened(win));
        if (safeMode || shell == null && focused == null) setFocus(win);
        if (safeMode) safeLayout();
        requestTick(0);
        return c;
    }

    /** New pixels of a legacy window (whole frame, the damaged rectangles in its pixels). */
    public synchronized void legacyFrame(UiConnection c, byte[] pixels, int format, int[] palette, int w, int h, List<Box> rects) {
        Surface s = c.surfaces.get(1);
        if (s == null || c.closed || w <= 0 || h <= 0) return;
        boolean whole = rects == null;
        if (s.bw != w || s.bh != h || s.format != format) {
            if (w > display.deviceW() || h > display.deviceH()) return;
            s.setBuffer(w, h, format, 1);
            whole = true;
        }
        if (palette != null) {
            s.palette = palette.clone();
            whole = true;
        }
        if (whole) {
            Px.convert(pixels, format, s.palette, w, h, 0, 0, w, h, s.argb);
            s.damageAll = true;
        } else {
            for (Box b : rects) {
                Px.convert(pixels, format, s.palette, w, h, b.x0(), b.y0(), b.w(), b.h(), s.argb);
            }
        }
        s.pixelVersion++;
        boolean first = !s.mapped;
        s.mapped = true;
        if (first || s.damageAll) {
            s.damageAll = false;
            damageSurface(s);
        } else {
            for (Box b : rects) damageSurfaceLu(s, b);
        }
        requestTick(0);
    }

    /** The window of a legacy connection, or 0. */
    public synchronized int windowOf(UiConnection c) {
        Surface s = c.surfaces.get(1);
        return s != null && s.window != null ? s.window.id : 0;
    }

    // =================================================================== shell messages

    private void place(UiProto.Place p) {
        UiWindow w = windows.get(p.window());
        if (w == null) return;
        damageWindow(w);
        w.place(p.x(), p.y(), p.w(), p.h(), p.transition(), host.nowMs());
        w.states = p.states();
        configureWindow(w);
        damageWindow(w);
        if (w.animating(host.nowMs())) requestTick(0);
    }

    private void configureWindow(UiWindow w) {
        Surface s = w.surface;
        if (w.cfgW == w.w && w.cfgH == w.h && w.cfgStates == w.states) return;
        w.cfgW = w.w;
        w.cfgH = w.h;
        w.cfgStates = w.states;
        if (s.conn.legacy != null) {
            s.states = w.states;
            return;
        }
        s.sendConfigure(w.w, w.h, w.states);
    }

    private void restack(List<Integer> ids) {
        List<UiWindow> order = new ArrayList<>();
        for (int id : ids) {
            UiWindow w = windows.get(id);
            if (w != null && !order.contains(w)) order.add(w);
        }
        // Windows the shell didn't name keep their relative order, below the named ones.
        List<UiWindow> rest = new ArrayList<>(stack);
        rest.removeAll(order);
        stack.clear();
        stack.addAll(rest);
        stack.addAll(order);
        damage.add(displayBox());
        requestTick(0);
    }

    /**
     * Bring the app whose process is {@code pid} to the front: on a phone, an app launched
     * again while it runs (telephonyd's incoming call, a notification) is raised, not started twice.
     * The shell activates its topmost mapped window (safe mode focuses it). False if it has none.
     */
    public synchronized boolean raiseApp(int pid) {
        UiWindow top = null;
        for (UiWindow w : stack) if (w.surface.conn.pid == pid && !w.surface.conn.closed && w.surface.mapped) top = w;
        if (top == null) return false;
        if (top == focused) return true;
        if (safeMode || shell == null) setFocus(top);
        else shell.queue(new UiProto.WindowRequest(top.id, UiProto.WindowAction.ACTIVATE, 0));
        requestTick(0);
        return true;
    }

    private void setFocus(UiWindow w) {
        if (w == focused) return;
        UiWindow old = focused;
        focused = w;
        if (old != null && !old.surface.conn.closed) {
            old.surface.conn.queue(new UiProto.Focus(old.surface.id, false));
            if (pointerLocked != null && pointerLocked.window == old) unlockPointer();
        }
        if (w != null) w.surface.conn.queue(new UiProto.Focus(w.surface.id, true));
        if (safeMode) safeLayout();
        host.focusChanged(old == null ? -1 : old.surface.conn.pid, w == null ? -1 : w.surface.conn.pid);
    }

    private void requestClose(UiWindow w) {
        if (w.surface.conn.legacy != null) {
            w.surface.conn.legacy.close();
        } else {
            w.surface.conn.queue(new UiProto.CloseRequested(w.surface.id));
        }
    }

    private void windowChanged(Surface s) {
        if (s.window != null && shell != null) {
            shell.queue(new UiProto.WindowChanged(s.window.id, s.title, s.minW, s.minH, s.maxW, s.maxH));
        }
        if (safeMode) damage.add(new Box(0, 0, display.deviceW(), SAFE_BAR_H * display.scale()));
    }

    private void pointerLock(UiConnection c, UiProto.PointerLock p) throws UiError {
        Surface s = surface(c, p.surface());
        if (!p.lock()) {
            if (pointerLocked == s) unlockPointer();
            return;
        }
        boolean ok = visible && s.window != null && s.window == focused;
        if (ok) pointerLocked = s;
        c.queue(new UiProto.PointerLockChanged(s.id, ok));
    }

    private void unlockPointer() {
        Surface s = pointerLocked;
        pointerLocked = null;
        if (s != null && !s.conn.closed) s.conn.queue(new UiProto.PointerLockChanged(s.id, false));
    }

    // =================================================================== safe mode

    private void enterSafeMode(String why, boolean restart) {
        if (closedForGood) return;
        if (!safeMode || restartAtMs < 0) safeReason = why;
        safeMode = true;
        host.log("safe mode: " + why);
        if (shellPid > 0) {
            host.kill(shellPid);
        }
        if (restart && restartAtMs < 0 && shellProgram != null && !shellProgram.isEmpty()) {
            long delay = RESTART_BACKOFF_MS[Math.min(backoff, RESTART_BACKOFF_MS.length - 1)];
            backoff++;
            restartAtMs = host.nowMs() + delay;
            requestTick(delay);
        }
        if (focused == null) setFocus(topmostShown());
        safeLayout();
    }

    private void leaveSafeMode() {
        if (!safeMode) return;
        safeMode = false;
        restartAtMs = -1;
        damage.add(displayBox());
        requestTick(0);
    }

    /** Built-in layout: the focused window maximized under a plain text bar (§8 "Safe mode"). */
    private void safeLayout() {
        if (!safeMode) return;
        UiWindow w = focused;
        if (w != null) {
            int lw = display.logicalW(), lh = display.logicalH();
            int st = UiProto.WindowStates.MAXIMIZED | UiProto.WindowStates.ACTIVATED;
            if (w.x != 0 || w.y != SAFE_BAR_H || w.w != lw || w.h != lh - SAFE_BAR_H || !w.placed || w.states != st) {
                w.place(0, SAFE_BAR_H, lw, lh - SAFE_BAR_H, null, host.nowMs());
                w.states = st;
                configureWindow(w);
            }
        }
        damage.add(displayBox());
        requestTick(0);
    }

    private UiWindow topmostShown() {
        for (int i = stack.size() - 1; i >= 0; i--) {
            UiWindow w = stack.get(i);
            if (w.visible) return w;
        }
        return null;
    }

    private void cycleFocus() {
        if (stack.isEmpty()) return;
        int i = focused == null ? -1 : stack.indexOf(focused);
        UiWindow next = stack.get((i + 1 + stack.size()) % stack.size());
        stack.remove(next);
        stack.add(next);
        setFocus(next);
        damage.add(displayBox());
        requestTick(0);
    }

    // =================================================================== input

    /**
     * A pointer event from a player, in device pixels. {@code kind}: the
     * protocol's {@link UiProto.PointerKind} code for move/down/up/scroll;
     * {@code button} the protocol Button code; {@code scroll} wheel steps.
     */
    public synchronized void pointer(UiProto.PointerKind kind, int x, int y, int held, UiProto.Button button, int scroll) {
        if (!visible) return;
        pointerX = x;
        pointerY = y;
        buttons = held;
        Surface target = grab != null ? grab : surfaceAt(x, y);
        if (kind == UiProto.PointerKind.DOWN) {
            dismissPopups(target);
            if (target != null && grab == null) grab = target;
            // A click in the focused app's window (or its popup) may be a paste click (§12).
            if (target != null && focused != null && target.conn == focused.surface.conn && target.conn.legacy == null) {
                pasteArmed = target.conn;
                pasteArmedAt = host.nowMs();
            }
            if (target != null && target.window != null && target.window != focused) {
                if (safeMode || shell == null) setFocus(target.window);
                else shell.queue(new UiProto.WindowRequest(target.window.id, UiProto.WindowAction.ACTIVATE, 0));
            }
        }
        if (target != hover && grab == null) {
            if (hover != null) deliverPointer(hover, UiProto.PointerKind.LEAVE, x, y, UiProto.Button.NONE, 0);
            hover = target;
            if (target != null) deliverPointer(target, UiProto.PointerKind.ENTER, x, y, UiProto.Button.NONE, 0);
        }
        if (target != null) deliverPointer(target, kind, x, y, button, scroll);
        if (kind == UiProto.PointerKind.UP && held == 0) {
            Surface g = grab;
            grab = null;
            Surface under = surfaceAt(x, y);
            if (g != null && under != g) {
                deliverPointer(g, UiProto.PointerKind.LEAVE, x, y, UiProto.Button.NONE, 0);
                hover = under;
                if (under != null) deliverPointer(under, UiProto.PointerKind.ENTER, x, y, UiProto.Button.NONE, 0);
            }
        }
        drainFails();
    }

    private void deliverPointer(Surface s, UiProto.PointerKind kind, int x, int y, UiProto.Button button, int scroll) {
        Geo g = geo(s, host.nowMs());
        if (g == null) return;
        if (s.isPixels()) {
            int bx = Math.floorDiv(x - g.ox, g.k), by = Math.floorDiv(y - g.oy, g.k);
            if (s.conn.legacy != null) {
                int cx = Math.max(0, Math.min(s.bw - 1, bx)), cy = Math.max(0, Math.min(s.bh - 1, by));
                s.conn.legacy.pointer(kind.code, cx, cy, buttons, button.code, scroll);
                return;
            }
            s.conn.queue(new UiProto.Pointer(s.id, kind, bx * s.pscale, by * s.pscale, buttons, button, 0, scroll, 0));
            return;
        }
        int lx = Math.floorDiv(x - g.ox, g.k), ly = Math.floorDiv(y - g.oy, g.k);
        int hit = Rasterizer.hitTest(s.tree.root(), lx, ly);
        s.conn.queue(new UiProto.Pointer(s.id, kind, lx, ly, buttons, button, 0, scroll, hit));
    }

    private void dismissPopups(Surface target) {
        for (Surface p : new ArrayList<>(popups)) {
            if (p.popupDone || !p.mapped) continue;
            boolean inside = false;
            for (Surface t = target; t != null; t = t.parent) if (t == p) inside = true;
            if (!inside) {
                p.popupDone = true;
                damageSurface(p);
                p.conn.queue(new UiProto.PopupDone(p.id));
            }
        }
    }

    /**
     * A key from a player as a USB HID usage with protocol mods. Returns true
     * if the compositor used it (a chord, Escape releasing pointer lock, safe
     * mode's switch-app chord) or delivered it to a window.
     */
    public synchronized boolean key(int hid, int mods, boolean down, boolean repeat) {
        if (!visible) return false;
        try {
            if (down) heldKeys.add(hid);
            else heldKeys.remove(hid);
            if (!down && swallowed.remove(hid)) return true;
            if (down && hid == HID_ESCAPE && pointerLocked != null) {
                unlockPointer();
                swallowed.add(hid);
                return true;
            }
            if (down && !repeat && safeMode && hid == HID_TAB && mods == UiProto.Mods.ALT) {
                cycleFocus();
                swallowed.add(hid);
                return true;
            }
            if (down && shell != null) {
                for (Map.Entry<Integer, int[]> e : chords.entrySet()) {
                    if (matches(e.getValue(), hid, mods)) {
                        if (!repeat) shell.queue(new UiProto.Chord(e.getKey()));
                        swallowed.add(hid);
                        return true;
                    }
                }
            }
            UiWindow w = focused;
            if (w == null || w.surface.conn.closed) return false;
            if (w.surface.conn.legacy != null) {
                w.surface.conn.legacy.key(hid, mods, down);
                return true;
            }
            if (down && isPasteKey(hid, mods)) {
                pasteArmed = w.surface.conn;
                pasteArmedAt = host.nowMs();
            }
            w.surface.conn.queue(new UiProto.Key(w.surface.id, hid & 0xFFFF, mods & UiProto.Mods.MASK, down, repeat));
            return true;
        } finally {
            drainFails();
        }
    }

    private boolean matches(int[] spec, int hid, int mods) {
        if ((mods & UiProto.Mods.MASK) != spec[0]) return false;
        boolean has = false;
        for (int i = 1; i < spec.length; i++) {
            if (spec[i] == hid) has = true;
            else if (!heldKeys.contains(spec[i])) return false;
        }
        return has;
    }

    /** Typed text (§7 Text) for the focused window. Returns true if a protocol window took it. */
    public synchronized boolean text(String s) {
        if (!visible || s.isEmpty()) return false;
        UiWindow w = focused;
        if (w == null || w.surface.conn.closed || w.surface.conn.legacy != null) return false;
        for (int i = 0; i < s.length(); ) {
            int end = i;
            int bytes = 0;
            while (end < s.length()) {
                int cp = s.codePointAt(end);
                int n = cp < 0x80 ? 1 : cp < 0x800 ? 2 : cp < 0x10000 ? 3 : 4;
                if (bytes + n > 256) break;
                bytes += n;
                end += Character.charCount(cp);
            }
            w.surface.conn.queue(new UiProto.Text(w.surface.id, s.substring(i, end)));
            i = end;
        }
        drainFails();
        return true;
    }

    /** The focused window's legacy sink, if a legacy program has focus (its keys go to its stdin). */
    public synchronized UiConnection.LegacySink focusedLegacy() {
        UiWindow w = focused;
        return visible && w != null && !w.surface.conn.closed ? w.surface.conn.legacy : null;
    }

    private void releaseInputLocked() {
        heldKeys.clear();
        swallowed.clear();
        grab = null;
        if (pointerLocked != null) unlockPointer();
    }

    /** The topmost shown surface under device point (x, y). */
    private Surface surfaceAt(int x, int y) {
        List<Item> items = drawOrder(host.nowMs());
        for (int i = items.size() - 1; i >= 0; i--) {
            Item it = items.get(i);
            if (it.surface != null && it.geo.frame.contains(x, y)) return it.surface;
        }
        return null;
    }

    // =================================================================== geometry and order

    private Geo geo(Surface s, long now) {
        return SceneRenderer.geo(display, s, now);
    }

    /** The scene as the shared renderer sees it (also what {@link SceneExporter} sends to clients). */
    SceneRenderer.Scene scene() {
        return new SceneRenderer.Scene(display, layers, stack, popups, safeMode, focused);
    }

    private List<Item> drawOrder(long now) {
        return SceneRenderer.drawOrder(scene(), now);
    }

    // =================================================================== damage

    private void damageSurface(Surface s) {
        Geo g = geo(s, host.nowMs());
        if (g != null) damage.add(g.frame.intersect(displayBox()));
        for (Surface p : popups) if (p.parent == s) damageSurface(p);
    }

    private void damageWindow(UiWindow w) {
        damageSurface(w.surface);
    }

    /** Damage in a surface's own units: lu for trees, buffer pixels times scale for pixel surfaces. */
    private void damageSurfaceLu(Surface s, Box b) {
        damage.add(SceneRenderer.surfaceDamage(display, s, b, host.nowMs()));
    }

    // =================================================================== ticking

    private long periodMs() {
        return Math.max(1, 1000 / refreshHz);
    }

    private void requestTick(long delayMs) {
        host.requestTick(Math.max(0, delayMs) * 1_000_000L);
    }

    /**
     * One refresh: deferred commits, animations, Frame events, the shell
     * watchdog and restarts, then composite the damage and present it.
     */
    public synchronized void tick() {
        if (closedForGood) return;
        long now = host.nowMs();
        try {
            preTick.run();
        } catch (RuntimeException e) {
            host.log("pre-tick failed: " + e);
        }
        // Commits merged by the tree-ops rate limit.
        boolean deferred = false;
        for (UiConnection c : new ArrayList<>(conns.values())) {
            for (Surface s : new ArrayList<>(c.surfaces.values())) {
                if (!s.commitDeferred || c.closed) continue;
                refill(c);
                if (c.opTokens < 0) {
                    deferred = true;
                    continue;
                }
                s.commitDeferred = false;
                try {
                    applyCommit(s);
                } catch (UiError e) {
                    failLater(c, e);
                }
            }
        }
        drainFails();
        // Animations damage their windows every refresh.
        boolean animating = false;
        for (UiWindow w : stack) {
            if (w.animating(now) || w.animating(now - periodMs() * 2)) {
                animating |= w.animating(now);
                damage.add(displayBox());
            }
        }
        // Frame pacing: only visible surfaces get Frame (§5). A timed producer gets it when its queue has room.
        boolean framesWaiting = false;
        long nowUs = DisplayClockNow();
        if (visible) {
            List<Item> order = drawOrder(now);
            for (UiConnection c : conns.values()) {
                for (Surface s : c.surfaces.values()) {
                    if (!s.frameRequested) continue;
                    if (timedActive(c, s, nowUs) && !timedRoom(c, s)) {
                        framesWaiting = true;
                        continue;
                    }
                    if (frameVisible(s, order)) {
                        s.frameRequested = false;
                        c.queue(new UiProto.Frame(s.id, now));
                    } else {
                        framesWaiting = true;
                    }
                }
            }
        }
        drainFails();
        // Shell watchdog and restarts.
        long next = Long.MAX_VALUE;
        if (shell != null) {
            if (shell.unreadAgeMs(now) > limits.shellTimeoutMs()) {
                enterSafeMode("the shell stopped answering", true);
                UiConnection dead = shell;
                disconnectLocked(dead, null);
            } else {
                next = Math.min(next, 1000);
            }
        } else if (shellPid > 0 && !safeMode) {
            if (now - shellStartedMs > limits.shellTimeoutMs()) enterSafeMode("the shell didn't connect", true);
            else next = Math.min(next, 500);
        }
        if (restartAtMs >= 0) {
            if (now >= restartAtMs) startShell();
            else next = Math.min(next, restartAtMs - now);
        }
        // Part A6: when every viewer draws the scene itself, the damage waits until someone needs pixels.
        if (visible && !damage.isEmpty()) {
            if (host.pixelsWanted()) composite(now);
            else compositesSkipped++;
        }
        if (exporter != null) exporter.afterTick(now);
        if (animating || deferred) next = Math.min(next, periodMs());
        if (next != Long.MAX_VALUE) requestTick(next);
    }

    private boolean frameVisible(Surface s, List<Item> order) {
        int idx = -1;
        for (int i = 0; i < order.size(); i++) if (order.get(i).surface == s) idx = i;
        if (idx < 0 || order.get(idx).alpha == 0) return false;
        Box f = order.get(idx).geo.frame.intersect(displayBox());
        if (f.isEmpty()) return false;
        for (int i = idx + 1; i < order.size(); i++) {
            Item it = order.get(i);
            if (it.surface != null && it.surface.isPixels() && it.alpha == 255 && it.surface.window != null
                    && it.geo.frame.contains(f)) {
                return false; // covered by an opaque window
            }
        }
        return true;
    }

    // =================================================================== compositing

    static Rasterizer.Resources resourcesOf(UiConnection c) {
        return new Rasterizer.Resources() {
            @Override public int[] pixels(int id) {
                UiResource r = c.resources.get(id);
                return r == null ? null : r.argb;
            }

            @Override public int width(int id) {
                UiResource r = c.resources.get(id);
                return r == null ? 0 : r.w;
            }

            @Override public int height(int id) {
                UiResource r = c.resources.get(id);
                return r == null ? 0 : r.h;
            }
        };
    }

    private void composite(long now) {
        List<Box> boxes = damage.take(displayBox());
        if (boxes.isEmpty()) return;
        SceneRenderer.composite(display, frame, rgba, boxes, drawOrder(now), safeBarText(), sf -> resourcesOf(sf.conn));
        frames++;
        host.present(rgba, display.deviceW(), display.deviceH(), boxes);
    }

    /** Safe mode's bar text (drawn by {@link SceneRenderer#drawSafeBar}). */
    String safeBarText() {
        String title = focused != null ? focused.surface.title : "no windows";
        return "SAFE MODE  " + safeReason + (restartAtMs >= 0 ? "; restarting it" : "") + "  |  " + title
                + "  |  Alt+Tab: next window";
    }

    // =================================================================== ePhone (P5)
    //
    // In-hand hotkeys and the OS switch-app key (UI_PROTOCOL §7, docs/gui/EPHONE.md),
    // lifecycle delivery for the phone (WillSuspend), permission prompts through the
    // shell (§10) and what probes need. Driven by PhoneUi.

    /** Chord id the shell gets for the ePhone's OS-level "switch app" key (§7; reserved, never bound by BindChord). */
    public static final int CHORD_SWITCH_APP = 0xEC01;

    /** A notification as it was posted (diagnostics, probes). */
    public record PostedNotification(int nid, String appId, String tag, String title, String body, long atMs) {}

    /** One hotkey claim: {@code appId} (pid) labelled slot {@code slot}. */
    public record HotkeyClaimInfo(int pid, String appId, int slot, String label, boolean focused) {}

    private void claimHotkey(UiConnection c, UiProto.HotkeyClaim h) {
        String[] slots = hotkeyClaims.computeIfAbsent(c.id, k -> new String[7]);
        slots[h.slot()] = h.label().isEmpty() ? null : h.label();
    }

    /**
     * Hotkey slot {@code slot} (1-6) went down or up on an ePhone in hand: a
     * {@code Hotkey} for the app whose window is open (focused), if it claimed the
     * slot; otherwise for the app with the active media session if it claimed the slot.
     * Returns the app id it went to, or null (nobody claimed it).
     */
    public synchronized String hotkey(int slot, boolean down) {
        if (slot < 1 || slot > 6) return null;
        UiWindow w = focused;
        UiConnection c = w == null || w.surface.conn.closed || w.surface.conn.legacy != null ? null : w.surface.conn;
        String[] slots = c == null ? null : hotkeyClaims.get(c.id);
        if (slots == null || slots[slot] == null) {
            // Not claimed on screen (or at home): the app holding the active media session gets
            // it, so Music's Play/Pause works from home and in the background.
            c = mediaHotkeyTarget(slot);
            if (c == null) return null;
        }
        c.queue(new UiProto.Hotkey(slot, down));
        drainFails();
        return c.appId == null ? "" : c.appId;
    }

    /** The connection of the active media session ({@link #activeMedia}) if it claimed {@code slot}. */
    private UiConnection mediaHotkeyTarget(int slot) {
        MediaEntry m = activeMedia();
        if (m == null) return null;
        for (Map.Entry<Integer, MediaEntry> e : media.entrySet()) {
            if (e.getValue() != m) continue;
            UiConnection mc = conns.get(e.getKey());
            String[] s = mc == null || mc.closed ? null : hotkeyClaims.get(mc.id);
            return s != null && s[slot] != null ? mc : null;
        }
        return null;
    }

    /** The OS "switch app" key: the shell switches to the next open app; without a shell the next window gets focus. */
    public synchronized void switchApp() {
        if (shell != null && !safeMode) shell.queue(new UiProto.Chord(CHORD_SWITCH_APP));
        else cycleFocus();
        drainFails();
    }

    /** Every hotkey claim of every open app (the phone's Controls screen). */
    public synchronized List<HotkeyClaimInfo> hotkeyClaims() {
        List<HotkeyClaimInfo> out = new ArrayList<>();
        UiConnection fc = focused == null ? null : focused.surface.conn;
        for (Map.Entry<Integer, String[]> e : hotkeyClaims.entrySet()) {
            UiConnection c = conns.get(e.getKey());
            if (c == null) continue;
            for (int s = 1; s <= 6; s++) {
                if (e.getValue()[s] != null) out.add(new HotkeyClaimInfo(c.pid, c.appId == null ? "" : c.appId, s, e.getValue()[s], c == fc));
            }
        }
        return out;
    }

    /** Queue {@code m} to every app connection (not the shell) whose program is in {@code pids}; how many got it. */
    public synchronized int sendToApp(java.util.function.IntPredicate pids, UiProto.Message m) {
        int n = 0;
        for (UiConnection c : new ArrayList<>(conns.values())) {
            if (c.closed || !c.hello || c.isShell() || c.legacy != null || !pids.test(c.pid)) continue;
            c.queue(m);
            n++;
        }
        drainFails();
        return n;
    }

    /** The program whose window has focus (the open app on a phone), or -1. */
    public synchronized int focusedPid() {
        return focused == null ? -1 : focused.surface.conn.pid;
    }

    /** Notifications posted so far (the last 64). */
    public synchronized List<PostedNotification> notifications() {
        return List.copyOf(posted);
    }


    // =================================================================== client rendering

    /** Attach (or detach, with null) the exporter that turns each tick's changes into scene patches. */
    public synchronized void setExporter(SceneExporter e) {
        exporter = e;
    }

    public synchronized SceneExporter exporter() {
        return exporter;
    }

    // =================================================================== P4: file picker and chooser

    /**
     * Show the "open with" chooser for {@code path} on behalf of {@code fromPid}
     * (0 = the system): the app manager's {@code open} found several handlers
     * and no default ({@code AppException.CHOOSE}). The choice goes to
     * {@link CompositorHost#openWith}.
     */
    public synchronized boolean startChooser(int fromPid, String path) {
        int parent = 0;
        for (UiConnection c : conns.values()) if (c.pid == fromPid && parent == 0) parent = windowOf(c);
        return dialogs.choose(fromPid, parent, path, (app, always) -> {
            if (app != null) host.openWith(path, app, always);
        });
    }

    /** A {@code picked} / {@code chosen} line on {@code /dev/uictl} from program {@code pid}. */
    public synchronized String systemAnswer(int pid, String line) {
        String r = dialogs.answer(pid, line);
        requestTick(0);
        return r;
    }

    public synchronized SystemDialogs dialogs() {
        return dialogs;
    }

    /**
     * Where the first text node showing exactly {@code text} is drawn in the
     * window titled {@code title}: its centre in device pixels, or null.
     * Scenarios click buttons by their label with this.
     */
    public synchronized int[] findText(String title, String text) {
        long now = host.nowMs();
        for (UiWindow w : stack) {
            if (!w.surface.title.equals(title) || w.surface.tree == null) continue;
            int[] r = w.rect(now);
            int[] at = findText(w.surface.tree.root(), text, 0, 0);
            if (at != null) {
                int s = display.scale();
                return new int[]{(r[0] + at[0]) * s, (r[1] + at[1]) * s};
            }
        }
        return null;
    }

    private static int[] findText(NodeTree.Node n, String text, int ox, int oy) {
        for (NodeTree.Node c : n.children()) {
            if (c.kind == UiProto.NodeKind.TEXT && text.equals(c.text)) {
                int tw = c.w > 0 ? c.w : 4 * text.length();
                return new int[]{ox + c.x + tw / 2, oy + c.y + Math.max(c.h, 8) / 2};
            }
            if (c.kind == UiProto.NodeKind.GROUP) {
                int[] r = findText(c, text, ox + c.x, oy + c.y);
                if (r != null) return r;
            }
        }
        return null;
    }

    // =================================================================== inspection (tests, diagnostics)

    /** What a window looks like right now. */
    public record WindowInfo(int id, int pid, String appId, String title, int x, int y, int w, int h, int states,
                             boolean placed, boolean mapped, boolean visible, boolean focused, boolean legacy, boolean shell) {}

    public synchronized List<WindowInfo> windows() {
        List<WindowInfo> out = new ArrayList<>();
        long now = host.nowMs();
        for (UiWindow w : stack) {
            int[] r = w.rect(now);
            out.add(new WindowInfo(w.id, w.surface.conn.pid, w.surface.conn.appId, w.surface.title, r[0], r[1], r[2], r[3],
                    w.states, w.placed, w.surface.mapped, w.visible, w == focused, w.surface.conn.legacy != null,
                    w.surface.conn.isShell()));
        }
        return out;
    }

    /** The launcher entry that goes back to the console instead of starting an app. */
    public static final String CONSOLE_APP = "console";

    /**
     * The player's real clipboard, sent by their client right before a paste
     * key: the paste that follows reads it.
     */
    public synchronized void setClipboardText(String text) {
        clipboardMime = "text/plain;charset=utf-8";
        clipboard = text.getBytes(java.nio.charset.StandardCharsets.UTF_8);
    }

    /** What the clipboard holds (tests, diagnostics), as text. */
    public synchronized String clipboardText() {
        return clipboardMime.isEmpty() ? "" : new String(clipboard, java.nio.charset.StandardCharsets.UTF_8);
    }

    /** The current theme's name (what the shell last set). */
    public synchronized String themeName() {
        return theme.name();
    }

    /** Every text node's string in a window's tree, in tree order (tests, scenario probes). */
    public synchronized List<String> textsOf(int windowId) {
        UiWindow w = windows.get(windowId);
        List<String> out = new ArrayList<>();
        if (w == null || w.surface.tree == null) return out;
        NodeTree.collectTexts(w.surface.tree.root(), out);
        return out;
    }

    /** The drawn text nodes of a window with their positions (surface lu): checks read a text grid back. */
    public synchronized List<NodeTree.PlacedText> placedTextsOf(int windowId) {
        UiWindow w = windows.get(windowId);
        List<NodeTree.PlacedText> out = new ArrayList<>();
        if (w == null || w.surface.tree == null) return out;
        NodeTree.collectPlacedTexts(w.surface.tree.root(), out);
        return out;
    }

    /**
     * Where the text node showing {@code text} (exactly, or containing it if
     * {@code contains}) is drawn in window {@code windowId}: the device pixel at
     * its centre, or null (also when another surface covers that pixel).
     * Scenario probes click there, the way a player would.
     */
    public synchronized int[] locateText(int windowId, String text, boolean contains) {
        UiWindow w = windows.get(windowId);
        if (w == null || w.surface.tree == null || !SceneRenderer.shown(scene(), w)) return null;
        int[] lu = NodeTree.findText(w.surface.tree.root(), text, contains);
        if (lu == null) return null;
        Geo g = geo(w.surface, host.nowMs());
        if (g == null) return null;
        return onTop(w.surface, new int[]{g.ox + lu[0] * g.k, g.oy + lu[1] * g.k});
    }

    /** {@code at}, if {@code s} is what a player sees (and clicks) there; otherwise null (covered). */
    private int[] onTop(Surface s, int[] at) {
        return surfaceAt(at[0], at[1]) == s ? at : null;
    }

    /** Every text string in the shell's layer surfaces titled {@code title} (the taskbar), in tree order. */
    public synchronized List<String> layerTexts(String title) {
        List<String> out = new ArrayList<>();
        for (Surface s : layers) if (s.title.equals(title) && s.tree != null) NodeTree.collectTexts(s.tree.root(), out);
        return out;
    }

    /** Like {@link #locateText} for the shell's layer surface titled {@code title} (the taskbar). */
    public synchronized int[] locateLayerText(String title, String text, boolean contains) {
        for (Surface s : layers) {
            if (!s.title.equals(title) || s.tree == null || !s.mapped) continue;
            int[] lu = NodeTree.findText(s.tree.root(), text, contains);
            if (lu == null) continue;
            Geo g = geo(s, host.nowMs());
            if (g != null) return onTop(s, new int[]{g.ox + lu[0] * g.k, g.oy + lu[1] * g.k});
        }
        return null;
    }

    /**
     * The clickable (hit-testable) nodes of the shell's layer titled {@code title}, as
     * {hitId, device x, device y} at their centres, left to right: checks find icons with no
     * text (the desktop's tray) by what appeared or went away.
     */
    public synchronized List<int[]> layerHits(String title) {
        List<int[]> out = new ArrayList<>();
        for (Surface s : layers) {
            if (!s.title.equals(title) || s.tree == null || !s.mapped) continue;
            Geo g = geo(s, host.nowMs());
            if (g == null) continue;
            List<int[]> lu = new ArrayList<>();
            NodeTree.collectHits(s.tree.root(), lu);
            for (int[] h : lu) out.add(new int[]{h[0], g.ox + h[1] * g.k, g.oy + h[2] * g.k});
        }
        out.sort((a, b) -> Integer.compare(a[1], b[1]));
        return out;
    }

    /** Like {@link #locateText} for any open popup surface. */
    public synchronized int[] locatePopupText(String text, boolean contains) {
        for (Surface s : popups) {
            if (s.tree == null || !s.mapped || s.popupDone) continue;
            int[] lu = NodeTree.findText(s.tree.root(), text, contains);
            if (lu == null) continue;
            Geo g = geo(s, host.nowMs());
            if (g != null) return onTop(s, new int[]{g.ox + lu[0] * g.k, g.oy + lu[1] * g.k});
        }
        return null;
    }

    public synchronized boolean safeMode() {
        return safeMode;
    }

    public synchronized String safeReason() {
        return safeReason;
    }

    public synchronized int shellPid() {
        return shellPid;
    }

    public synchronized boolean shellConnected() {
        return shell != null;
    }

    public synchronized int connectionCount() {
        return (int) conns.values().stream().filter(c -> c.legacy == null).count();
    }

    public synchronized long framesComposited() {
        return frames;
    }

    /** A copy of the composited frame as straight RGBA bytes. */
    public synchronized byte[] frameRgba() {
        compositePending();
        return rgba.clone();
    }

    /** Ticks whose composite waited because nobody needed the pixels (Part A6; diagnostics). */
    private long compositesSkipped;

    public synchronized long compositesSkipped() {
        return compositesSkipped;
    }

    /**
     * Composite damage left over from ticks that skipped it (nobody needed
     * the pixels then). Something needs them now: a tile viewer appeared, a
     * screenshot or a check reads the frame.
     */
    public synchronized void compositePending() {
        if (visible && !damage.isEmpty()) composite(host.nowMs());
    }

    /** Composite everything now (tests): the whole display is damaged first. */
    public synchronized void repaintAll() {
        damage.add(displayBox());
        boolean v = visible;
        visible = true;
        composite(host.nowMs());
        visible = v;
    }
}
