package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.EcmConfig;
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.apps.AppEvents;
import com.example.evanscomputermod.computer.apps.AppException;
import com.example.evanscomputermod.computer.apps.AppInstance;
import com.example.evanscomputermod.computer.apps.AppManager;
import com.example.evanscomputermod.computer.apps.LifecyclePolicy;
import com.example.evanscomputermod.computer.apps.PermissionPrompter;
import com.example.evanscomputermod.computer.ui.proto.UiProto;
import com.example.evanscomputermod.wasm.WasmManager;
import org.jetbrains.annotations.Nullable;

import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.TimeUnit;

/**
 * The ePhone side of a computer's GUI (ROADMAP P5, docs/gui/EPHONE.md "Phone
 * shell and lifecycle"). Version-neutral: the 1.21.1 phone host drives it
 * ({@code phone/PhoneApps}) and JUnit drives it with a portrait test host.
 *
 * <ul>
 *   <li><b>Always GUI.</b> {@link UiManager} creates one at boot when the host's
 *       display is portrait; the compositor then starts at once with the phone
 *       shell {@value #SHELL} pinned ({@link #config}), and the console switch
 *       chord does nothing.</li>
 *   <li><b>Apps.</b> {@code Launch} of an installed app goes through the app
 *       manager ({@link #launch}), so apps have their identity and lifecycle.
 *       The built-in apps (Notes, GBA, Terminal) are installed into {@code /apps} at boot
 *       from the mod's resources ({@link #provision}).</li>
 *   <li><b>iPhone lifecycle</b> ({@link LifecyclePolicy#PHONE}). The app whose
 *       window has focus is the foreground app and runs; every other app is
 *       suspended ({@code WillSuspend} -> {@code SaveState} within the deadline
 *       -> its process group paused). When the phone goes to the hotbar or
 *       away ({@link #willSleep}) the foreground app is suspended too, and the
 *       computer's sleep waits for that; back in hand ({@link #wake}) it
 *       resumes. Refresh wakes and exception modes come from the app manager's
 *       {@code BackgroundScheduler} (its own 1 s timer). {@code audio} starts by
 *       itself when an app that declared it writes to the phone's speaker
 *       ({@link #audioActivity}) and stops 2 s after the last write.</li>
 *   <li><b>Cellular.</b> While a framed modem and a SIM are in, the phone runs
 *       {@code celld --phone} ({@link #cellular}, started and stopped here), which
 *       reports its bars and carrier through {@code /dev/uictl cell <bars> <carrier>}
 *       ({@link #cellReport}); the status bar shows them.</li>
 *   <li><b>Input.</b> In-hand hotkey slots go to the open app
 *       ({@link #hotkey}), the switch-app key to the shell ({@link #switchApp}).</li>
 *   <li><b>Prompts.</b> It is the app manager's {@link PermissionPrompter}
 *       (the shell's dialog, through {@link Compositor#requestPermission}) and
 *       {@link AppEvents} ({@code WillSuspend} over the apps' UI connections).</li>
 * </ul>
 */
public final class PhoneUi implements AppEvents, PermissionPrompter {
    /** The phone shell (program {@code phone}). */
    public static final String SHELL = "dev.ecm.phone";
    /** Apps that come with every phone: app id, the program in the server's wasm-bin that is its entry. */
    static final String[][] BUILTINS = {{"dev.ecm.notes", "notes"}, {"dev.ecm.gba", "gba-app"}, {"dev.ecm.terminal", "terminal"},
            {"dev.ecm.settings", "settings"}, {"dev.ecm.files", "files"},
            {"dev.ecm.dialer", "dialer"}, {"dev.ecm.messages", "messages"}, {"dev.ecm.contacts", "contacts"}};
    /** A celld report older than this no longer counts (the daemon reports every 5 s). */
    static final long CELL_REPORT_MS = 12_000;
    /** celld is not started again sooner than this after it exited. */
    static final long CELLD_RESTART_MS = 5_000;
    /** How long an app may go without a window before it is suspended like any other background app. */
    static final long WINDOWLESS_GRACE_MS = 5_000;
    /** An audio exception mode ends this long after the app's last write to the speaker. */
    static final long AUDIO_IDLE_MS = 2_000;
    /** Extra time past the save deadline the computer keeps running before it sleeps. */
    static final long SLEEP_GRACE_EXTRA_MS = 300;

    /** What the phone hardware reports, for the status bar ({@code /dev/uictl phone}). */
    public interface Status {
        /** Battery in percent. */
        double battery();

        /** "awake", "asleep" or "off". */
        String state();

        /** Wi-Fi signal bars 0-4, or -1 without Wi-Fi. */
        int wifiBars();

        /** Cellular signal bars 0-4, or -1 without a modem. */
        int cellBars();
    }

    private static final ScheduledExecutorService TIMER = Executors.newSingleThreadScheduledExecutor(r -> {
        Thread t = new Thread(r, "ECM-Phone-Timer");
        t.setDaemon(true);
        return t;
    });

    private final ComputerInstance computer;
    private final UiManager ui;
    private final AppManager apps;
    private final ExecutorService worker;
    private volatile Status status = new Status() {
        @Override public double battery() { return 100; }
        @Override public String state() { return "awake"; }
        @Override public int wifiBars() { return -1; }
        @Override public int cellBars() { return -1; }
    };
    private volatile boolean awake = true;
    private volatile AppInstance foreground;
    /** Process group -> wall time of its last write to the speaker. */
    private final Map<Integer, Long> audioAt = new ConcurrentHashMap<>();
    /** Process group -> wall time of its last read of the microphone (the MIC pill). */
    private final Map<Integer, Long> micAt = new ConcurrentHashMap<>();
    /** A microphone read longer ago than this no longer lights the MIC pill. */
    static final long MIC_IDLE_MS = 1_000;
    /** Media sessions by app id (the Now Playing card; AirPods double-tap). */
    private final Map<String, UiProto.MediaInfo> media = new ConcurrentHashMap<>();
    private final List<String> events = new CopyOnWriteArrayList<>();
    private volatile ScheduledFuture<?> sweeper;
    private volatile boolean closed;
    /** Cellular v2: whether celld should run (a framed modem and a SIM are in), its pid, its last report. */
    private volatile boolean celldWanted;
    private volatile int celldPid = -1;
    private volatile int telephonydPid = -1;
    private volatile long telephonydStartedAt;
    private volatile boolean volte;
    private volatile String msisdn = "";
    @Nullable
    private volatile String ringingWho;
    private volatile long celldStartedAt, celldCheckedAt;
    private volatile int cellBars = -1;
    private volatile String cellCarrier = "";
    private volatile long cellReportAt;

    PhoneUi(ComputerInstance computer, UiManager ui) {
        this.computer = computer;
        this.ui = ui;
        this.apps = computer.getAppManager();
        this.worker = Executors.newSingleThreadExecutor(r -> {
            Thread t = new Thread(r, "ECM-Phone-" + computer.idString());
            t.setDaemon(true);
            return t;
        });
    }

    /** At boot: iPhone policy, prompts and events through the GUI, built-in apps, the background timer. */
    void boot() {
        if (apps == null) return;
        apps.setPolicy(LifecyclePolicy.PHONE);
        apps.setAppEvents(this);
        apps.setPrompter(this);
        // The background limit never stops what is on screen.
        apps.setForegroundProbe(i -> awake && i == foreground);
        provision();
        apps.reindex();
        apps.background().start();
        sweeper = TIMER.scheduleWithFixedDelay(this::sweepSafely, 500, 500, TimeUnit.MILLISECONDS);
    }

    /** The phone's ui config: {@code /etc/ui.toml} with the phone shell pinned and autostart on. */
    UiConfig config(UiConfig file) {
        return new UiConfig(true, SHELL, file.shellArgs(), file.theme(), "off", "");
    }

    public void setStatus(Status s) {
        if (s != null) status = s;
    }

    public AppManager apps() {
        return apps;
    }

    /** What happened, in order ("suspend id", "resume id", "refresh id", "audio on/off id"; tests and diagnostics). */
    public List<String> events() {
        return List.copyOf(events);
    }

    /** The app on screen now, or null (home screen, or the phone sleeps with nothing open). */
    @Nullable
    public AppInstance foreground() {
        return foreground;
    }

    private void event(String e) {
        events.add(e);
        if (events.size() > 200) events.remove(0);
        EvansComputerMod.LOGGER.info("[phone {}] {}", computer.idString(), e);
    }

    // ------------------------------------------------------------ apps

    /** Start installed app {@code appId} (one instance per app: a running one is reused). Its pid, or -1 if it isn't an app. */
    int launch(String appId, List<String> args) {
        if (apps == null || apps.manifest(appId) == null) return -1;
        for (AppInstance i : apps.running()) {
            if (i.appId().equals(appId) && i.pid() > 0) return i.pid();
        }
        try {
            AppInstance i = apps.launch(appId, args);
            saveSession();
            return i.pid();
        } catch (AppException e) {
            EvansComputerMod.LOGGER.warn("[phone] can't launch {}: {}", appId, e.getMessage());
            return -1;
        }
    }

    private void saveSession() {
        List<String> ids = new ArrayList<>();
        for (AppInstance i : apps.running()) if (!ids.contains(i.appId())) ids.add(i.appId());
        apps.states().saveSession(ids);
    }

    /** Install the built-in apps into {@code /apps} (again when the mod ships a newer manifest or program). */
    void provision() {
        Path root = computer.storagePath();
        Path bin = WasmManager.getWasmBinPath();
        if (root == null) return;
        for (String[] b : BUILTINS) {
            String id = b[0];
            try {
                byte[] toml = resource("/assets/evanscomputermod/ecm_apps/" + id + "/app.toml");
                Path wasm = bin == null ? null : bin.resolve(b[1] + ".wasm");
                if (toml == null || wasm == null || !Files.isRegularFile(wasm)) continue;
                Path dir = root.resolve("apps").resolve(id);
                Path manifest = dir.resolve("app.toml"), entry = dir.resolve("main.wasm");
                boolean same = Files.isRegularFile(manifest) && java.util.Arrays.equals(Files.readAllBytes(manifest), toml)
                        && Files.isRegularFile(entry) && Files.size(entry) == Files.size(wasm);
                if (same) continue;
                Files.createDirectories(dir);
                Files.copy(wasm, entry, java.nio.file.StandardCopyOption.REPLACE_EXISTING);
                byte[] icon = resource("/assets/evanscomputermod/ecm_apps/" + id + "/icon.png");
                if (icon != null) Files.write(dir.resolve("icon.png"), icon);
                Files.write(manifest, toml);
                Files.writeString(dir.resolve(".pkg-origin"), "builtin\n", StandardCharsets.UTF_8);
                Files.createDirectories(root.resolve("var/apps").resolve(id));
                // (a newer manifest's new permission, Bluetooth for one, is granted on update too, unless it was refused)
                if (SYSTEM_GRANTED.containsKey(id) && apps != null) {
                    // The phone's own Settings: what it declares is allowed once at install (revocable), as SystemApps does.
                    for (String perm : SYSTEM_GRANTED.get(id)) {
                        if (apps.permissions().decision(id, perm) == null) apps.permissions().setDecision(id, perm, true);
                    }
                }
            } catch (IOException e) {
                EvansComputerMod.LOGGER.warn("[phone] can't install the built-in app {}: {}", id, e.toString());
            }
        }
    }

    /**
     * Built-ins whose declared permissions are allowed at install: the phone's Settings (Wi-Fi, Cellular, Apps),
     * its Files (the documents folder) and Phone, Messages and Contacts.
     */
    static final Map<String, List<String>> SYSTEM_GRANTED = Map.of("dev.ecm.settings", List.of("network", "radio", "cellular", "packages", "peripherals:bluetooth", "mixer"),
            "dev.ecm.files", List.of("files:documents"),
            // Phone, Messages and Contacts (docs/TELEPHONY.md): what they declare, so a call or a text never waits on a prompt.
            "dev.ecm.dialer", List.of("phone", "contacts", "notifications"),
            "dev.ecm.messages", List.of("sms", "contacts", "files:documents", "microphone", "notifications"),
            "dev.ecm.contacts", List.of("contacts", "sms", "files:documents"));

    @Nullable
    private static byte[] resource(String path) throws IOException {
        try (InputStream in = PhoneUi.class.getResourceAsStream(path)) {
            return in == null ? null : in.readAllBytes();
        }
    }

    // ------------------------------------------------------------ lifecycle

    /** The compositor moved focus (under its lock): the foreground app changes on the worker. */
    void focusChanged(int oldPid, int newPid) {
        if (closed) return;
        worker.execute(() -> foregroundIs(newPid > 0 ? apps.instanceOf(newPid) : null));
    }

    private void foregroundIs(@Nullable AppInstance next) {
        AppInstance prev = foreground;
        foreground = next;
        if (next != null && awake && next.state() == AppInstance.State.SUSPENDED) {
            apps.resume(next);
            event("resume " + next.appId());
        }
        if (prev != next) enforce(false);
    }

    /**
     * Every app but the foreground one is suspended. With {@code all} false an
     * app started in the last few seconds is left alone (it is still opening its
     * window, which the shell will focus).
     */
    private void enforce(boolean all) {
        if (apps == null) return;
        long now = System.currentTimeMillis();
        for (AppInstance i : apps.running()) {
            if (i.state() != AppInstance.State.RUNNING || (awake && i == foreground)) continue;
            if (!all && now - i.startedMs() < WINDOWLESS_GRACE_MS) continue;
            apps.suspend(i);
            event("suspend " + i.appId());
        }
    }

    /**
     * The phone is about to stop running (in hand -> hotbar or away): suspend
     * the running apps. Returns how long the computer should keep running so
     * they can save their state (ms).
     */
    public long willSleep() {
        awake = false;
        if (apps == null) return 0;
        long n = apps.running().stream().filter(i -> i.state() == AppInstance.State.RUNNING).count();
        if (n == 0) return 0;
        worker.execute(() -> {
            if (!awake) enforce(true);
        });
        return Math.min(2_000, n * EcmConfig.appsSaveStateDeadlineMs() + SLEEP_GRACE_EXTRA_MS);
    }

    /** Back in hand: the foreground app runs again. */
    public void wake() {
        awake = true;
        worker.execute(() -> {
            AppInstance fg = foreground;
            if (awake && fg != null && fg.state() == AppInstance.State.SUSPENDED) {
                apps.resume(fg);
                event("resume " + fg.appId());
            }
        });
    }

    public boolean isAwake() {
        return awake;
    }

    /**
     * A program read the phone's microphone: while its app is hidden (or the phone is away) the
     * status bar shows MIC, so the player knows a background app hears them.
     */
    public void micActivity(int pid) {
        if (apps == null || pid <= 0) return;
        AppInstance i = apps.instanceOf(pid);
        if (i != null) micAt.put(i.group(), System.currentTimeMillis());
    }

    /** Apps that read the microphone just now while not on screen (the MIC pill). */
    public java.util.List<String> hiddenMicUsers() {
        java.util.List<String> out = new ArrayList<>();
        if (apps == null) return out;
        long now = System.currentTimeMillis();
        for (AppInstance i : apps.running()) {
            Long at = micAt.get(i.group());
            if (at == null || now - at > MIC_IDLE_MS) continue;
            if (awake && i == foreground) continue;
            if (!out.contains(i.appId())) out.add(i.appId());
        }
        return out;
    }

    /** The compositor saw an app's media session change (state none: it ended). */
    void mediaChanged(int pid, String appId, UiProto.MediaInfo info) {
        if (info.state() == UiProto.MediaState.NONE) media.remove(appId);
        else media.put(appId, info);
    }

    /**
     * Double-tap on the AirPods (or another button with no screen): pause what plays, or play
     * what was paused last. Returns the app it went to, or null when nothing has a media session.
     */
    @Nullable
    public String mediaToggle() {
        Compositor c = ui.compositor();
        Compositor.MediaEntry m = c == null ? null : c.activeMedia();
        if (m == null) {
            event("media toggle -> nobody");
            return null;
        }
        String action = m.playing() ? "pause" : "play";
        c.mediaAction(m.pid(), action, 0);
        event("media " + action + " -> " + m.appId());
        return m.appId();
    }

    /** A program wrote to the phone's speaker: its app is playing audio (the {@code audio} exception mode). */
    public void audioActivity(int pid) {
        if (apps == null || pid <= 0) return;
        AppInstance i = apps.instanceOf(pid);
        if (i == null) return;
        audioAt.put(i.group(), System.currentTimeMillis());
        if (!i.activeModes().contains("audio") && apps.background().startMode(i, "audio")) event("audio on " + i.appId());
    }

    private void sweepSafely() {
        try {
            sweep();
        } catch (Throwable t) {
            EvansComputerMod.LOGGER.warn("[phone] sweep failed", t);
        }
    }

    private void sweep() {
        if (closed || apps == null) return;
        long now = System.currentTimeMillis();
        for (AppInstance i : apps.running()) {
            Long at = audioAt.get(i.group());
            if (i.activeModes().contains("audio") && !apps.background().isExplicit(i, "audio") && (at == null || now - at > AUDIO_IDLE_MS)) {
                apps.background().stopMode(i, "audio");
                audioAt.remove(i.group());
                event("audio off " + i.appId());
            }
        }
        // Apps that opened no window in time, or were left running: background apps are suspended.
        if (awake) worker.execute(() -> enforce(false));
    }

    // ------------------------------------------------------------ input

    /** In-hand hotkey slot (1-6): to the open app if it claimed the slot. Its app id, or null. */
    @Nullable
    public String hotkey(int slot, boolean down) {
        Compositor c = ui.compositor();
        String app = c == null ? null : c.hotkey(slot, down);
        event("hotkey " + slot + (down ? " down" : " up") + " -> " + (app == null ? "nobody" : app));
        return app;
    }

    /** The OS switch-app key: the shell shows the next open app. */
    public void switchApp() {
        Compositor c = ui.compositor();
        if (c != null) c.switchApp();
        event("switch app");
    }

    // ------------------------------------------------------------ AppEvents

    @Override
    public boolean willSuspend(AppInstance app, int deadlineMs) {
        return ui.deliverWillSuspend(app, deadlineMs);
    }

    /** Open with on a phone: {@code OpenFile} like on a Terminal. */
    @Override
    public void openFile(AppInstance app, String path) {
        ui.deliverOpenFile(app, path);
    }

    @Override
    public void refreshWake(AppInstance app, long budgetMs) {
        event("refresh " + app.appId() + " " + budgetMs + "ms");
    }

    @Override
    public void backgroundModesChanged(AppInstance app, Set<String> activeModes) {
        event("modes " + app.appId() + " " + activeModes);
        ui.deliverAppModes(app, activeModes);
    }

    @Override
    public void backgroundModeResult(AppInstance app, String mode, boolean granted) {
        event("background " + mode + " " + (granted ? "granted" : "refused") + " " + app.appId());
        ui.deliverBackgroundModeResult(app, mode, granted);
    }

    /** The background limit stopped {@code stopped}: a notification names it ("Music was paused to let Discord run"). */
    @Override
    public void backgroundLimitReached(AppInstance stopped, AppInstance by) {
        String a = stopped.manifest().name(), b = by.manifest().name();
        event("limit " + stopped.appId() + " by " + by.appId());
        ui.systemNotify(stopped.appId(), "background-limit", a + " was paused", a + " was paused to let " + b + " run");
    }

    // ------------------------------------------------------------ PermissionPrompter

    @Override
    @Nullable
    public CompletableFuture<Answer> prompt(Request request) {
        // The same path as a Terminal's: the phone shell's dialog through Compositor.requestPermission.
        return ui.promptViaShell(request);
    }

    // ------------------------------------------------------------ cellular (celld)

    /**
     * Server tick: should celld run (a framed modem and a SIM card are in)? Starts
     * {@code celld --phone} when it should and isn't running (again 5 s after it
     * exited), stops it when the modem or the SIM goes. Spawning happens on the worker.
     */
    public void cellular(boolean want) {
        if (closed) return;
        long now = System.currentTimeMillis();
        if (want == celldWanted && now - celldCheckedAt < 1_000) return;
        celldCheckedAt = now;
        celldWanted = want;
        try {
            worker.execute(this::syncCelld);
        } catch (java.util.concurrent.RejectedExecutionException e) {
            // closing
        }
    }

    private void syncCelld() {
        var pm = computer.getProcessManager();
        int pid = celldPid;
        boolean alive = pid > 0 && pm != null && pm.isAlive(pid);
        if (celldWanted && !alive) {
            if (pid > 0) {
                event("celld exited (pid " + pid + ")");
                celldPid = -1;
            }
            if (System.currentTimeMillis() - celldStartedAt < CELLD_RESTART_MS) return;
            celldStartedAt = System.currentTimeMillis();
            int p = computer.spawnDetached("celld", List.of("--phone"));
            celldPid = p;
            event(p > 0 ? "celld start pid " + p : "celld: no program");
        } else if (!celldWanted && pid > 0) {
            if (alive && pm != null) pm.kill(pid);
            celldPid = -1;
            cellBars = -1;
            cellCarrier = "";
            event("celld stop pid " + pid);
        }
        // telephonyd (calls, texts) runs beside celld (it waits for the IMS bearer)
        int tp = telephonydPid;
        boolean talive = tp > 0 && pm != null && pm.isAlive(tp);
        if (celldWanted && !talive) {
            if (tp > 0) {
                event("telephonyd exited (pid " + tp + ")");
                telephonydPid = -1;
                volte = false;
                ringingWho = null;
            }
            if (System.currentTimeMillis() - telephonydStartedAt < CELLD_RESTART_MS) return;
            telephonydStartedAt = System.currentTimeMillis();
            int p = computer.spawnDetached("telephonyd", List.of("--phone"));
            telephonydPid = p;
            event(p > 0 ? "telephonyd start pid " + p : "telephonyd: no program");
        } else if (!celldWanted && tp > 0) {
            if (talive && pm != null) pm.kill(tp);
            telephonydPid = -1;
            volte = false;
            msisdn = "";
            ringingWho = null;
            event("telephonyd stop pid " + tp);
        }
    }

    /** The running celld's pid, or -1. */
    public int celldPid() {
        return celldPid;
    }

    /** The running telephonyd's pid (the IMS client, docs/TELEPHONY.md), or -1. */
    public int telephonydPid() {
        return telephonydPid;
    }

    /** {@code /dev/uictl volte <0|1> <number|->} from telephonyd. */
    void volteReport(boolean on, String number) {
        volte = on;
        msisdn = number == null || number.equals("-") ? "" : number;
    }

    /** Registered for VoLTE (the status bar's {@code VoLTE}). */
    public boolean volte() {
        return volte && telephonydPid > 0;
    }

    /** The phone's number as telephonyd last said ("" before it registered). */
    public String msisdn() {
        return msisdn;
    }

    /** {@code /dev/uictl ring}: the caller's name while a call rings, null otherwise. */
    void ringing(@Nullable String who) {
        ringingWho = who;
    }

    /** The caller of the ringing call (for the holder's HUD message), or null. */
    @Nullable
    public String ringingWho() {
        return telephonydPid > 0 ? ringingWho : null;
    }

    /** {@code /dev/uictl cell <bars> <carrier|->} from celld. */
    void cellReport(int bars, String carrier) {
        cellBars = Math.max(0, Math.min(4, bars));
        cellCarrier = carrier == null || carrier.equals("-") ? "" : carrier;
        cellReportAt = System.currentTimeMillis();
    }

    private boolean cellFresh() {
        return celldPid > 0 && System.currentTimeMillis() - cellReportAt < CELL_REPORT_MS;
    }

    /** Bars celld last reported (0 while it hasn't, or its report is stale). */
    public int celldBars() {
        return cellFresh() ? cellBars : 0;
    }

    /** The carrier celld is on ("" when on none). */
    public String celldCarrier() {
        return cellFresh() ? cellCarrier : "";
    }

    // ------------------------------------------------------------ status for the shell

    /**
     * {@code /dev/uictl phone}: what the phone shell's status bar and Controls
     * screen show, one item per line:
     * <pre>
     * state awake
     * battery 87.5
     * wifi 3                       (bars 0-4, -1 none)
     * cell -1
     * carrier cell-red             (the carrier celld is on, or -)
     * foreground dev.ecm.notes     (or -)
     * mode dev.ecm.music audio     (one line per app and active exception mode)
     * hotkey dev.ecm.notes 1 * Add note   (* = the open app, - = another)
     * </pre>
     */
    String statusText() {
        Status s = status;
        StringBuilder sb = new StringBuilder();
        sb.append("state ").append(s.state()).append('\n');
        sb.append("battery ").append(String.format(java.util.Locale.ROOT, "%.1f", s.battery())).append('\n');
        sb.append("wifi ").append(s.wifiBars()).append('\n');
        sb.append("cell ").append(s.cellBars()).append('\n');
        String carrier = celldCarrier();
        sb.append("carrier ").append(carrier.isEmpty() ? "-" : carrier).append('\n');
        sb.append("volte ").append(volte() ? 1 : 0).append('\n');
        sb.append("number ").append(msisdn.isEmpty() ? "-" : msisdn).append('\n');
        AppInstance fg = foreground;
        sb.append("foreground ").append(fg == null ? "-" : fg.appId()).append('\n');
        if (apps != null) {
            apps.background().activeModes().forEach((id, modes) -> {
                for (String m : modes) sb.append("mode ").append(id).append(' ').append(m).append('\n');
            });
        }
        for (String id : hiddenMicUsers()) sb.append("mic ").append(id).append('\n');
        media.forEach((id, m) -> sb.append("media ").append(id).append(' ').append(m.state().name().toLowerCase(java.util.Locale.ROOT)).append('\n'));
        Compositor c = ui.compositor();
        if (c != null) {
            for (Compositor.HotkeyClaimInfo h : c.hotkeyClaims()) {
                sb.append("hotkey ").append(h.appId()).append(' ').append(h.slot()).append(' ')
                        .append(h.focused() ? '*' : '-').append(' ').append(h.label()).append('\n');
            }
        }
        return sb.toString();
    }

    void close() {
        closed = true;
        int pid = celldPid;
        var pm = computer.getProcessManager();
        if (pid > 0 && pm != null) pm.kill(pid);
        int tp = telephonydPid;
        if (tp > 0 && pm != null) pm.kill(tp);
        ScheduledFuture<?> f = sweeper;
        if (f != null) f.cancel(false);
        worker.shutdownNow();
    }
}
