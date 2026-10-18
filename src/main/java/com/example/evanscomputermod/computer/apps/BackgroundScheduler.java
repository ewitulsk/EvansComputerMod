package com.example.evanscomputermod.computer.apps;

import com.example.evanscomputermod.EcmConfig;
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.computer.wasi.ProcessManager;
import org.jetbrains.annotations.Nullable;

import java.util.Map;
import java.util.Set;
import java.util.TreeMap;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.TimeUnit;

/**
 * ePhone background rules (UI_PROTOCOL.md §11, docs/gui/APPS.md "Background"),
 * built on the pause API: an app is a process group, suspended = group paused.
 *
 * <ul>
 *   <li><b>Refresh wakes.</b> A suspended app whose manifest asks for
 *       {@code background = { refresh = true }} and that holds
 *       {@code background:refresh} gets a wake every {@link #intervalMs()}
 *       (config {@code apps.refreshIntervalMinutes}, default 5 min): its group
 *       is resumed (and the computer woken with {@code wakeFor} if it sleeps)
 *       for at most {@link #budgetMs()} (default 2 s) of wall time, then paused
 *       again. {@code BackgroundDone} ends the window early. One window at a
 *       time; apps that are due wait for the next {@link #tick}.</li>
 *   <li><b>Wakes.</b> {@link #wakeNow} gives a suspended app such a window right away
 *       (data on one of its wake sockets, {@code SO_ECM_WAKE}; a modem page), at most
 *       once per {@code apps.wakeMinIntervalMs} per app.</li>
 *   <li><b>Background modes</b> ({@code audio}, {@code call}, {@code radio},
 *       {@code transfer}, {@code navigation}): while a declared mode is active
 *       ({@link #startMode}: the app's {@code BackgroundMode} message, or its speaker
 *       writes for {@code audio}) and {@code background:<mode>} is allowed, a suspended
 *       app runs duty-cycled: resumed for {@code period * share / 100} of every
 *       {@link #DUTY_PERIOD_MS}, the share being {@code apps.backgroundCpuPercent.<mode>}
 *       (10 % of a core; 25 % for a call). There is no per-process CPU accounting, so the
 *       cap is wall-clock time a group may run.</li>
 *   <li><b>First-time prompt.</b> When an app with an undecided {@code background:<mode>}
 *       is hidden while the mode is on, the shell asks once ("Keep Discord running in the
 *       background for calls?"); the answer is remembered as {@code background:<mode>} and
 *       sent to the app as {@code BackgroundModeResult}.</li>
 *   <li><b>Limit.</b> At most {@code apps.maxBackgroundApps} (2) apps may have modes on at
 *       once; when one more starts, the app whose mode started longest ago is stopped
 *       (its modes end, it stays suspended) and {@link AppEvents#backgroundLimitReached}
 *       names it.</li>
 * </ul>
 * The phone host drives it: {@link LifecyclePolicy#PHONE}, then {@link #tick}
 * from its server tick (or {@link #start} for an own timer), and
 * {@link #setSleepControl} so wakes can wake a sleeping computer. On a Terminal
 * ({@link LifecyclePolicy#PC}) apps keep running anyway: modes are only recorded (the
 * desktop's tray shows them).
 */
public final class BackgroundScheduler implements AutoCloseable {

    /** Duty-cycle period for exception modes. */
    public static final long DUTY_PERIOD_MS = 100;

    /** Wakes a sleeping computer for a while (ComputerInstance.wakeFor). */
    public interface SleepControl {
        boolean isAsleep();

        /** Wake for up to {@code ms}; the returned handle ends the window early. */
        Runnable wakeFor(long ms);
    }

    /** One refresh window. */
    public static final class Window {
        public final AppInstance app;
        /** "refresh", or what woke it ("socket", "page"). */
        public final String reason;
        public final long startNanos = System.nanoTime();
        volatile long endNanos;
        /** "budget", "done", "resumed", "exited" once over. */
        volatile String endedBy;
        ScheduledFuture<?> timer;
        Runnable wake;

        Window(AppInstance app, String reason) {
            this.app = app;
            this.reason = reason;
        }

        public boolean isOver() {
            return endedBy != null;
        }

        @Nullable
        public String endedBy() {
            return endedBy;
        }

        /** How long the app was allowed to run, in ms (so far, if still open). */
        public long ranMs() {
            long end = endedBy != null ? endNanos : System.nanoTime();
            return (end - startNanos) / 1_000_000L;
        }
    }

    private static final ScheduledExecutorService TIMER = java.util.concurrent.Executors.newScheduledThreadPool(1, r -> {
        Thread t = new Thread(r, "ECM-App-Background");
        t.setDaemon(true);
        return t;
    });

    private final AppManager apps;
    private final ProcessManager pm;
    private volatile SleepControl sleep;
    private volatile long intervalMs = -1, budgetMs = -1;
    private volatile int cpuPercent = -1;
    private volatile int maxApps = -1;
    private volatile Window current;
    private final Map<Integer, Long> lastRefresh = new ConcurrentHashMap<>();
    private final Map<Integer, Long> lastWake = new ConcurrentHashMap<>();
    /** Wakes that came while another window was open: group -> reason. */
    private final Map<Integer, String> pendingWakes = new ConcurrentHashMap<>();
    private final Map<Integer, ScheduledFuture<?>> dutyCycles = new ConcurrentHashMap<>();
    private volatile ScheduledFuture<?> ticker;
    /** What happened, for tests and diagnostics ("wake dev.ecm.discord socket", "limit dev.ecm.music by dev.ecm.discord"). */
    private final java.util.List<String> log = new java.util.concurrent.CopyOnWriteArrayList<>();

    BackgroundScheduler(AppManager apps, ProcessManager pm) {
        this.apps = apps;
        this.pm = pm;
    }

    public void setSleepControl(@Nullable SleepControl sleep) {
        this.sleep = sleep;
    }

    public long intervalMs() {
        return intervalMs >= 0 ? intervalMs : EcmConfig.appsRefreshIntervalMinutes() * 60_000L;
    }

    public long budgetMs() {
        return budgetMs >= 0 ? budgetMs : EcmConfig.appsRefreshBudgetMs();
    }

    /** The CPU share (percent) of background {@code mode}. */
    public int cpuPercent(String mode) {
        return cpuPercent >= 0 ? cpuPercent : EcmConfig.appsBackgroundCpuPercent(mode);
    }

    public int maxApps() {
        return maxApps > 0 ? maxApps : EcmConfig.appsMaxBackgroundApps();
    }

    /** Override the config (tests, or a phone with its own settings); negative = back to the config. */
    public void configure(long intervalMs, long budgetMs, int cpuPercent) {
        this.intervalMs = intervalMs;
        this.budgetMs = budgetMs;
        this.cpuPercent = cpuPercent;
    }

    /** Tests: the background app limit instead of {@code apps.maxBackgroundApps} (0 = the config). */
    public void setMaxApps(int n) {
        this.maxApps = n;
    }

    /** Run {@link #tick} on an own timer (every second) instead of from the host. */
    public synchronized void start() {
        if (ticker == null) ticker = TIMER.scheduleWithFixedDelay(() -> tick(System.currentTimeMillis()), 1, 1, TimeUnit.SECONDS);
    }

    public java.util.List<String> log() {
        return java.util.List.copyOf(log);
    }

    private void note(String e) {
        log.add(e);
        if (log.size() > 200) log.remove(0);
        EvansComputerMod.LOGGER.info("[apps] {}", e);
    }

    // ------------------------------------------------------------------ refresh wakes

    /** May this app get refresh wakes? (asks for them and holds background:refresh) */
    public boolean refreshAllowed(AppInstance i) {
        return i.manifest().backgroundRefresh() && apps.check(i, "background:refresh", false);
    }

    /** Start due refresh wakes (PHONE policy only). Call regularly with the wall clock. */
    public void tick(long nowMs) {
        if (apps.policy() != LifecyclePolicy.PHONE) return;
        Window w = current;
        if (w != null && !w.isOver()) return;
        // A wake that had to wait for another window goes first.
        for (var e : pendingWakes.entrySet()) {
            AppInstance i = byGroup(e.getKey());
            pendingWakes.remove(e.getKey());
            if (i != null && open(i, e.getValue()) != null) return;
        }
        for (AppInstance i : apps.running()) {
            if (i.state() != AppInstance.State.SUSPENDED || !refreshAllowed(i)) continue;
            long last = lastRefresh.computeIfAbsent(i.group(), g -> nowMs);
            if (nowMs - last >= intervalMs()) {
                lastRefresh.put(i.group(), nowMs);
                refreshNow(i);
                return;
            }
        }
    }

    @Nullable
    private AppInstance byGroup(int group) {
        for (AppInstance i : apps.running()) if (i.group() == group) return i;
        return null;
    }

    /**
     * Give a suspended app one refresh window now. Returns it, or null if the
     * app may not refresh, isn't suspended, or another window is open.
     */
    @Nullable
    public Window refreshNow(AppInstance i) {
        return open(i, "refresh");
    }

    /**
     * Something arrived for a suspended app ({@code reason}: "socket" for a wake socket,
     * "page" for a modem page): give it a refresh window now, so it can read the data and
     * post a notification. At most once per {@code apps.wakeMinIntervalMs} per app; needs
     * {@code background:refresh}. If another app's window is open, the wake follows it.
     * Returns the window, or null (not suspended, not allowed, too soon, or queued).
     */
    @Nullable
    public Window wakeNow(AppInstance i, String reason) {
        if (apps.policy() != LifecyclePolicy.PHONE || i.state() != AppInstance.State.SUSPENDED || !refreshAllowed(i)) return null;
        long now = System.currentTimeMillis();
        Long last = lastWake.get(i.group());
        if (last != null && now - last < EcmConfig.appsWakeMinIntervalMs()) return null;
        Window open = current;
        if (open != null && !open.isOver()) {
            if (open.app == i) return open;
            pendingWakes.putIfAbsent(i.group(), reason);
            TIMER.schedule(() -> tick(System.currentTimeMillis()), Math.max(50, budgetMs() / 4), TimeUnit.MILLISECONDS);
            return null;
        }
        lastWake.put(i.group(), now);
        Window w = open(i, reason);
        if (w != null) note("wake " + i.appId() + " " + reason);
        return w;
    }

    @Nullable
    private synchronized Window open(AppInstance i, String reason) {
        Window open = current;
        if (open != null && !open.isOver()) return null;
        if (i.state() != AppInstance.State.SUSPENDED || !refreshAllowed(i)) return null;
        Window w = new Window(i, reason);
        current = w;
        long budget = budgetMs();
        SleepControl s = sleep;
        if (s != null && s.isAsleep()) w.wake = s.wakeFor(budget + 50);
        pm.resumeGroup(i.group());
        try {
            apps.events().refreshWake(i, budget);
        } catch (RuntimeException e) {
            EvansComputerMod.LOGGER.warn("[apps] refreshWake to {} failed: {}", i.appId(), e.toString());
        }
        w.timer = TIMER.schedule(() -> end(w, "budget"), budget, TimeUnit.MILLISECONDS);
        return w;
    }

    /** The open refresh window, or null. */
    @Nullable
    public Window currentWindow() {
        Window w = current;
        return w != null && !w.isOver() ? w : null;
    }

    void onBackgroundDone(AppInstance i) {
        Window w = current;
        if (w != null && w.app == i) end(w, "done");
    }

    /** End {@code i}'s window, if open, without pausing it (it was resumed to the foreground). */
    void endRefresh(AppInstance i, String why) {
        Window w = current;
        if (w != null && w.app == i) finish(w, why, false);
    }

    private void end(Window w, String why) {
        finish(w, why, true);
    }

    private synchronized void finish(Window w, String why, boolean pause) {
        if (w.isOver()) return;
        if (pause && w.app.state() == AppInstance.State.SUSPENDED && !runsInBackground(w.app)) pm.pauseGroup(w.app.group());
        w.endNanos = System.nanoTime();
        w.endedBy = why;
        if (w.timer != null) w.timer.cancel(false);
        if (w.wake != null) w.wake.run();
        EvansComputerMod.LOGGER.debug("[apps] refresh of {} ended by {} after {} ms", w.app.appId(), why, w.ranMs());
        if (!pendingWakes.isEmpty()) TIMER.execute(() -> tick(System.currentTimeMillis()));
    }

    // ------------------------------------------------------------------ background modes

    /** The remembered (or this run's) answer for {@code background:<mode>}, or null while undecided. */
    @Nullable
    Boolean decision(AppInstance i, String mode) {
        return apps.decisionOf(i, "background:" + mode);
    }

    /** May {@code i} run while hidden for {@code mode} (declared and allowed)? */
    public boolean allowed(AppInstance i, String mode) {
        return i.manifest().backgroundModes().contains(mode) && Boolean.TRUE.equals(decision(i, mode));
    }

    /** Does a suspended {@code i} run duty-cycled now (some active mode is allowed)? */
    boolean runsInBackground(AppInstance i) {
        for (String m : i.activeModes) if (allowed(i, m)) return true;
        return false;
    }

    /**
     * {@code app} is actually doing {@code mode} (audio playing, a call, a download): while
     * it is hidden it keeps running, capped, once {@code background:<mode>} is allowed (the
     * player is asked the first time it is hidden with the mode on). Returns false if the
     * mode isn't declared or the player said no.
     */
    public boolean startMode(AppInstance i, String mode) {
        if (!i.manifest().backgroundModes().contains(mode)) return false;
        if (apps.policy() != LifecyclePolicy.PHONE) {
            // A Terminal keeps apps running anyway: the mode only tells the desktop (close to tray).
            if (i.activeModes.add(mode)) {
                i.modeStartedMs.put(mode, System.currentTimeMillis());
                notifyModes(i);
            }
            return true;
        }
        Boolean d = decision(i, mode);
        if (Boolean.FALSE.equals(d)) return false;
        if (!i.activeModes.add(mode)) return true;
        i.modeStartedMs.put(mode, System.currentTimeMillis());
        enforceLimit(i);
        dutyCycles.computeIfAbsent(i.group(), g -> TIMER.scheduleAtFixedRate(() -> dutySlice(i), 0, DUTY_PERIOD_MS, TimeUnit.MILLISECONDS));
        notifyModes(i);
        if (d == null && i.state() == AppInstance.State.SUSPENDED) askFirstTime(i, mode);
        return true;
    }

    /**
     * {@code BackgroundMode{mode, on}} from the app: start or stop the mode, and answer with
     * {@code BackgroundModeResult} when the answer is known (now if decided, else once the
     * player answered the first-time prompt).
     */
    public void requested(AppInstance i, String mode, boolean on) {
        if (!on) {
            i.explicitModes.remove(mode);
            stopMode(i, mode);
            return;
        }
        boolean ok = startMode(i, mode);
        if (ok) i.explicitModes.add(mode);
        Boolean d = apps.policy() == LifecyclePolicy.PHONE ? decision(i, mode) : Boolean.TRUE;
        if (!ok) result(i, mode, false);
        else if (d != null) result(i, mode, d);
    }

    private void result(AppInstance i, String mode, boolean granted) {
        try {
            apps.events().backgroundModeResult(i, mode, granted);
        } catch (RuntimeException e) {
            EvansComputerMod.LOGGER.warn("[apps] backgroundModeResult to {} failed: {}", i.appId(), e.toString());
        }
    }

    /** The app was just suspended (hidden, or the phone went to sleep): ask for any undecided active mode. */
    void onSuspended(AppInstance i) {
        if (apps.policy() != LifecyclePolicy.PHONE) return;
        for (String m : i.activeModes) {
            if (decision(i, m) == null) askFirstTime(i, m);
        }
    }

    /** "Keep Discord running in the background for calls?" once; the answer is remembered. */
    private void askFirstTime(AppInstance i, String mode) {
        if (!i.promptingModes.add(mode)) return;
        note("ask " + i.appId() + " background:" + mode);
        apps.askAsync(i, "background:" + mode).whenComplete((allow, err) -> {
            i.promptingModes.remove(mode);
            boolean yes = err == null && Boolean.TRUE.equals(allow);
            note("answer " + i.appId() + " background:" + mode + " " + (yes ? "allow" : "deny"));
            result(i, mode, yes);
            if (!yes) stopMode(i, mode);
        });
    }

    /** More apps than {@link #maxApps()} have modes on: stop the one whose mode started longest ago. */
    private void enforceLimit(AppInstance starting) {
        int max = maxApps();
        java.util.List<AppInstance> busy = new java.util.ArrayList<>();
        for (AppInstance a : apps.running()) if (!a.activeModes.isEmpty()) busy.add(a);
        while (busy.size() > max) {
            AppInstance oldest = null;
            long oldestAt = Long.MAX_VALUE;
            for (AppInstance a : busy) {
                if (a == starting || a.state() == AppInstance.State.RUNNING && apps.isForeground(a)) continue;
                long at = a.modeStartedMs.values().stream().mapToLong(Long::longValue).min().orElse(Long.MAX_VALUE);
                if (at < oldestAt) {
                    oldestAt = at;
                    oldest = a;
                }
            }
            if (oldest == null) return;
            busy.remove(oldest);
            note("limit " + oldest.appId() + " by " + starting.appId());
            stopAllModes(oldest);
            if (oldest.state() == AppInstance.State.SUSPENDED) pm.pauseGroup(oldest.group());
            try {
                apps.events().backgroundLimitReached(oldest, starting);
            } catch (RuntimeException e) {
                EvansComputerMod.LOGGER.warn("[apps] backgroundLimitReached failed: {}", e.toString());
            }
        }
    }

    /** Did the app turn {@code mode} on itself (BackgroundMode)? Then only BackgroundMode off ends it. */
    public boolean isExplicit(AppInstance i, String mode) {
        return i.explicitModes.contains(mode);
    }

    public void stopMode(AppInstance i, String mode) {
        i.explicitModes.remove(mode);
        if (!i.activeModes.remove(mode)) return;
        i.modeStartedMs.remove(mode);
        if (i.activeModes.isEmpty()) stopDuty(i);
        notifyModes(i);
    }

    void stopAllModes(AppInstance i) {
        if (i.activeModes.isEmpty()) return;
        i.activeModes.clear();
        i.explicitModes.clear();
        i.modeStartedMs.clear();
        stopDuty(i);
        notifyModes(i);
    }

    private void stopDuty(AppInstance i) {
        ScheduledFuture<?> f = dutyCycles.remove(i.group());
        if (f != null) f.cancel(false);
        Window w = current;
        boolean inWindow = w != null && !w.isOver() && w.app == i;
        if (i.state() == AppInstance.State.SUSPENDED && !inWindow) pm.pauseGroup(i.group());
    }

    /** The share a suspended app runs at: the largest of its allowed modes' (0 = none allowed). */
    int shareOf(AppInstance i) {
        int share = 0;
        for (String m : i.activeModes) if (allowed(i, m)) share = Math.max(share, cpuPercent(m));
        return share;
    }

    /** One duty period: run a suspended app for its slice, then pause it again. */
    private void dutySlice(AppInstance i) {
        if (i.state() != AppInstance.State.SUSPENDED || i.activeModes.isEmpty()) return;
        Window w = current;
        if (w != null && !w.isOver() && w.app == i) return; // a refresh window is already running it
        int share = shareOf(i);
        if (share <= 0) return; // waiting for the player's answer
        long slice = Math.max(1, DUTY_PERIOD_MS * share / 100);
        if (slice >= DUTY_PERIOD_MS) {
            pm.resumeGroup(i.group());
            return;
        }
        SleepControl s = sleep;
        Runnable wake = s != null && s.isAsleep() ? s.wakeFor(slice) : null;
        pm.resumeGroup(i.group());
        TIMER.schedule(() -> {
            if (i.state() == AppInstance.State.SUSPENDED) {
                Window cw = current;
                if (cw == null || cw.isOver() || cw.app != i) pm.pauseGroup(i.group());
            }
            if (wake != null) wake.run();
        }, slice, TimeUnit.MILLISECONDS);
    }

    private void notifyModes(AppInstance i) {
        try {
            apps.events().backgroundModesChanged(i, i.activeModes());
        } catch (RuntimeException e) {
            EvansComputerMod.LOGGER.warn("[apps] backgroundModesChanged failed: {}", e.toString());
        }
    }

    /** Status-bar view: app id -> active exception modes (only apps with any). */
    public Map<String, Set<String>> activeModes() {
        Map<String, Set<String>> out = new TreeMap<>();
        for (AppInstance i : apps.running()) {
            if (!i.activeModes.isEmpty()) out.computeIfAbsent(i.appId(), k -> new java.util.TreeSet<>()).addAll(i.activeModes);
        }
        return out;
    }

    @Override
    public void close() {
        ScheduledFuture<?> t = ticker;
        if (t != null) t.cancel(false);
        dutyCycles.values().forEach(f -> f.cancel(false));
        dutyCycles.clear();
        Window w = current;
        if (w != null && w.timer != null) w.timer.cancel(false);
    }
}
