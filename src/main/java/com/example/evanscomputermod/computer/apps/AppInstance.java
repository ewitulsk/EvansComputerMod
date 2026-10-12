package com.example.evanscomputermod.computer.apps;

import java.io.ByteArrayOutputStream;
import java.util.Collections;
import java.util.LinkedHashSet;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;

/**
 * One running app: a process group (docs/gui/APPS.md, "Identity"). The root
 * process is what {@link AppManager#launch} started; programs the app has the
 * kernel start for it (lines typed into its shell session) join the same
 * group, so they share its identity and permissions. The instance lives until
 * every member has exited.
 */
public final class AppInstance {

    public enum State { RUNNING, SUSPENDED, EXITED }

    /** Bytes of the root process's output kept for {@code app_ctl output}. */
    static final int OUTPUT_CAP = 64 * 1024;

    private final AppManifest manifest;
    private final int group;
    volatile int rootPid = -1;
    private final long startedMs = System.currentTimeMillis();
    volatile State state = State.RUNNING;
    volatile Integer exitCode;
    /** Answers given with "don't remember": this run only. */
    final Map<String, Boolean> sessionAnswers = new ConcurrentHashMap<>();
    private final Set<String> denials = Collections.synchronizedSet(new LinkedHashSet<>());
    final Set<String> activeModes = ConcurrentHashMap.newKeySet();
    /** Set while the app manager waits for this app's SaveState. */
    volatile CompletableFuture<byte[]> pendingSave;
    private final ByteArrayOutputStream output = new ByteArrayOutputStream();
    private int outputDropped;

    AppInstance(AppManifest manifest, int group) {
        this.manifest = manifest;
        this.group = group;
    }

    public String appId() {
        return manifest.id();
    }

    public AppManifest manifest() {
        return manifest;
    }

    /** The process group every process of this app belongs to. */
    public int group() {
        return group;
    }

    /** The process the app manager launched (its main.wasm or the Python interpreter). */
    public int pid() {
        return rootPid;
    }

    public State state() {
        return state;
    }

    /** Exit code of the root process once it has exited, else null. */
    public Integer exitCode() {
        return exitCode;
    }

    public long startedMs() {
        return startedMs;
    }

    /** Permissions this run was denied (in order), for {@code pkg run} and Settings. */
    public Set<String> denials() {
        synchronized (denials) {
            return new LinkedHashSet<>(denials);
        }
    }

    void denied(String permission) {
        denials.add(permission);
    }

    /** Background exception modes active right now. */
    public Set<String> activeModes() {
        return Set.copyOf(activeModes);
    }

    synchronized void appendOutput(byte[] b, int n) {
        int over = output.size() + n - OUTPUT_CAP;
        if (over > 0) {
            byte[] keep = output.toByteArray();
            output.reset();
            int from = Math.min(over, keep.length);
            output.write(keep, from, keep.length - from);
            outputDropped += from;
        }
        output.write(b, 0, n);
    }

    /** Take up to {@code max} bytes of buffered output (the rest stays). */
    public synchronized byte[] takeOutput(int max) {
        byte[] all = output.toByteArray();
        int n = Math.min(max, all.length);
        output.reset();
        output.write(all, n, all.length - n);
        byte[] out = new byte[n];
        System.arraycopy(all, 0, out, 0, n);
        return out;
    }

    @Override
    public String toString() {
        return manifest.id() + "[group " + group + ", pid " + rootPid + ", " + state + "]";
    }
}
