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
}
