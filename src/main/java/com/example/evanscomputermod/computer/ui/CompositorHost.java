package com.example.evanscomputermod.computer.ui;

import java.util.List;

/**
 * What a {@link Compositor} needs from the computer it runs on. The real
 * one is {@link UiManager} (a computer's programs, its Terminal display and a
 * shared scheduler); tests supply their own to replay sessions with a manual
 * clock and no game.
 */
public interface CompositorHost {
    /** Start a program detached from the console (shell, Launch); its pid, or -1 if it can't run. */
    int spawn(String program, List<String> args);

    /** Kill a program (an unresponsive shell). */
    void kill(int pid);

    /**
     * {@code Launch{app_id, args}} from program {@code fromPid} (the shell, an
     * app; 0 = the compositor itself, e.g. a notification relaunch): an
     * installed app starts through the app manager with its identity; other
     * programs as {@link #spawn}. The pid, or -1. Called under the compositor's lock.
     */
    default int launch(int fromPid, String appId, List<String> args) {
        return spawn(appId, args);
    }

    /**
     * {@code Open{path}} from program {@code fromPid}: open the file with its
     * default handler, or show the chooser ({@link Compositor#startChooser}).
     * Called under the compositor's lock: do the work elsewhere.
     */
    default void open(int fromPid, String path) {}

    /**
     * May {@code pid} post a notification ({@code notifications} permission for
     * apps)? Answer on {@code then}, now or later from another thread (the check
     * may prompt; it must not run under the compositor's lock).
     */
    default void checkNotify(int pid, java.util.function.Consumer<Boolean> then) {
        then.accept(true);
    }

    /** The app id to report for a program (§3 Welcome, §8 WindowOpened): the app manager's, else its program name. */
    String appId(int pid);

    /** Composited pixels changed: {@code rgba} is the whole frame, {@code damage} what changed (device px). */
    void present(byte[] rgba, int w, int h, List<Box> damage);

    /**
     * Does anything need the composited pixels right now (DISPLAY_PIPELINE_SPEC.md
     * Part A6)? False while every viewer draws the scene client-side and
     * nothing else asked for pixels lately: the compositor then keeps the
     * damage and composites it when someone asks (a tile viewer, a
     * screenshot, a headless check through {@link Compositor#frameRgba}).
     */
    default boolean pixelsWanted() {
        return true;
    }

    /** A timed frame was queued or flushed (CommitAt, QueueFlush): stream it to clients now. */
    default void timedQueued() {}

    /** Call {@link Compositor#tick()} after about {@code delayNanos} (coalesced; earlier requests win). */
    void requestTick(long delayNanos);

    /** Monotonic time in milliseconds (animations, frame times, timeouts). */
    long nowMs();

    /** A message for the log / diagnostics. */
    default void log(String msg) {}

    /** An app put something on the clipboard (copy): mirror text to the player's real clipboard. */
    default void clipboardChanged(String mime, byte[] data) {}

    /** The desktop asked to go back to the console (its launcher's Console item). */
    default void showConsole() {}

    // App platform wiring (P4/P5): the computer's app manager answers these; tests use the defaults.

    /** Saved state to hand an app right after Welcome ({@code Restore}, §11), or null. */
    default byte[] restoreFor(int pid) {
        return null;
    }

    /** {@code SaveState} from {@code pid}: true if the app manager kept it (it was an app). */
    default boolean saveState(int pid, byte[] data) {
        return false;
    }

    /** {@code BackgroundDone} from {@code pid}. */
    default void backgroundDone(int pid) {}

    /**
     * {@code BackgroundMode{mode, on}} from {@code pid}: the app manager starts or stops the
     * mode and answers with {@code BackgroundModeResult}. Called under the compositor's lock:
     * do the work elsewhere.
     */
    default void backgroundMode(int pid, String mode, boolean on) {}

    /** An app's media session changed (state none: it ended). Called under the compositor's lock. */
    default void mediaSessionChanged(int pid, String appId, com.example.evanscomputermod.computer.ui.proto.UiProto.MediaInfo info) {}

    /** Keyboard focus moved between programs' windows (-1 = none); called under the compositor's lock. */
    default void focusChanged(int oldPid, int newPid) {}

    /** P4: the file picker answered program {@code pid}: grant its app that one file. */
    default void filePicked(int pid, String path, boolean write) {}

    /** P4: the chooser's answer: open {@code path} with {@code appId} (and make it the default for the type). */
    default void openWith(String path, String appId, boolean always) {}
}
