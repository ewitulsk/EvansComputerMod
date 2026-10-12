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

    /** The app id to report for a program (§3 Welcome, §8 WindowOpened): its program name for now. */
    String appId(int pid);

    /** Composited pixels changed: {@code rgba} is the whole frame, {@code damage} what changed (device px). */
    void present(byte[] rgba, int w, int h, List<Box> damage);

    /** Call {@link Compositor#tick()} after about {@code delayNanos} (coalesced; earlier requests win). */
    void requestTick(long delayNanos);

    /** Monotonic time in milliseconds (animations, frame times, timeouts). */
    long nowMs();

    /** A message for the log / diagnostics. */
    default void log(String msg) {}

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

    /** Keyboard focus moved between programs' windows (-1 = none); called under the compositor's lock. */
    default void focusChanged(int oldPid, int newPid) {}

    /** P4: the file picker answered program {@code pid}: grant its app that one file. */
    default void filePicked(int pid, String path, boolean write) {}

    /** P4: the chooser's answer: open {@code path} with {@code appId} (and make it the default for the type). */
    default void openWith(String path, String appId, boolean always) {}
}
