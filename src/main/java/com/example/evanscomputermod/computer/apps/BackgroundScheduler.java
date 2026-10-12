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
 *   <li><b>Exception modes</b> ({@code audio}, {@code radio}, {@code transfer},
 *       {@code navigation}): while a declared and allowed mode is active
 *       ({@link #startMode}, called by whatever knows the app is really doing
 *       it), a suspended app runs duty-cycled: resumed for
 *       {@code period * cpuPercent / 100} of every {@link #DUTY_PERIOD_MS}
 *       (default 10 % of a core). There is no per-process CPU accounting, so
 *       the cap is wall-clock time a group may run; a group with several busy
 *       threads can use that share on each of them.</li>
 * </ul>
 * The phone host drives it: {@link LifecyclePolicy#PHONE}, then {@link #tick}
 * from its server tick (or {@link #start} for an own timer), and
 * {@link #setSleepControl} so wakes can wake a sleeping computer.
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
        public final long startNanos = System.nanoTime();
        volatile long endNanos;
        /** "budget", "done", "resumed", "exited" once over. */
        volatile String endedBy;
        ScheduledFuture<?> timer;
        Runnable wake;

        Window(AppInstance app) {
            this.app = app;
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
    private volatile Window current;
    private final Map<Integer, Long> lastRefresh = new ConcurrentHashMap<>();
    private final Map<Integer, ScheduledFuture<?>> dutyCycles = new ConcurrentHashMap<>();
    private volatile ScheduledFuture<?> ticker;

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

    public int cpuPercent() {
        return cpuPercent >= 0 ? cpuPercent : EcmConfig.appsBackgroundCpuPercent();
    }

    /** Override the config (tests, or a phone with its own settings); negative = back to the config. */
    public void configure(long intervalMs, long budgetMs, int cpuPercent) {
        this.intervalMs = intervalMs;
        this.budgetMs = budgetMs;
        this.cpuPercent = cpuPercent;
    }

    /** Run {@link #tick} on an own timer (every second) instead of from the host. */
    public synchronized void start() {
        if (ticker == null) ticker = TIMER.scheduleWithFixedDelay(() -> tick(System.currentTimeMillis()), 1, 1, TimeUnit.SECONDS);
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

    /**
     * Give a suspended app one refresh window now. Returns it, or null if the
     * app may not refresh, isn't suspended, or another window is open.
     */
    @Nullable
    public synchronized Window refreshNow(AppInstance i) {
        Window open = current;
        if (open != null && !open.isOver()) return null;
        if (i.state() != AppInstance.State.SUSPENDED || !refreshAllowed(i)) return null;
        Window w = new Window(i);
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
        if (pause && w.app.state() == AppInstance.State.SUSPENDED) pm.pauseGroup(w.app.group());
        w.endNanos = System.nanoTime();
        w.endedBy = why;
        if (w.timer != null) w.timer.cancel(false);
        if (w.wake != null) w.wake.run();
        EvansComputerMod.LOGGER.debug("[apps] refresh of {} ended by {} after {} ms", w.app.appId(), why, w.ranMs());
    }

    // ------------------------------------------------------------------ exception modes

    /**
     * {@code app} is actually doing {@code mode} (audio playing, a download
     * running): while it is suspended it keeps running, capped. Returns false
     * if the mode isn't declared or {@code background:<mode>} isn't allowed.
     */
    public boolean startMode(AppInstance i, String mode) {
        if (!i.manifest().backgroundModes().contains(mode)) return false;
        if (!apps.check(i, "background:" + mode, false)) return false;
        if (!i.activeModes.add(mode)) return true;
        dutyCycles.computeIfAbsent(i.group(), g -> TIMER.scheduleAtFixedRate(() -> dutySlice(i), 0, DUTY_PERIOD_MS, TimeUnit.MILLISECONDS));
        notifyModes(i);
        return true;
    }

    public void stopMode(AppInstance i, String mode) {
        if (!i.activeModes.remove(mode)) return;
        if (i.activeModes.isEmpty()) stopDuty(i);
        notifyModes(i);
    }

    void stopAllModes(AppInstance i) {
        if (i.activeModes.isEmpty()) return;
        i.activeModes.clear();
        stopDuty(i);
        notifyModes(i);
    }

    private void stopDuty(AppInstance i) {
        ScheduledFuture<?> f = dutyCycles.remove(i.group());
        if (f != null) f.cancel(false);
        if (i.state() == AppInstance.State.SUSPENDED) pm.pauseGroup(i.group());
    }

    /** One duty period: run a suspended app for its slice, then pause it again. */
    private void dutySlice(AppInstance i) {
        if (i.state() != AppInstance.State.SUSPENDED || i.activeModes.isEmpty()) return;
        Window w = current;
        if (w != null && !w.isOver() && w.app == i) return; // a refresh window is already running it
        long slice = Math.max(1, DUTY_PERIOD_MS * cpuPercent() / 100);
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
