package com.example.evanscomputermod.phone;

//? if <=1.21.1 {
import java.util.List;
import java.util.concurrent.CopyOnWriteArrayList;

/**
 * Hooks for the app platform's background scheduler (P4): it learns when a
 * phone changes state and can hold a sleep back briefly so apps get their
 * {@code WillSuspend} first. While a phone is {@link PhoneState#ASLEEP} the
 * scheduler gives it refresh windows with {@link PhoneHost#refreshWake(long)}
 * (a budgeted {@code ComputerInstance.wakeFor}) and picks which app runs in
 * them (process groups, {@code ProcessManager.resumeGroup}).
 */
public final class PhoneLifecycle {

    /** Server thread. */
    public interface Listener {
        /**
         * The phone is about to stop running (AWAKE to ASLEEP or OFF). Return how
         * long (ms, capped at {@link #MAX_GRACE_MS}) the computer should keep running
         * first, e.g. to deliver {@code WillSuspend{deadline_ms}}; 0 = at once.
         */
        default long willSleep(PhoneHost phone, PhoneState next) {
            return 0;
        }

        /** The state changed (after the computer was woken or put to sleep). */
        default void stateChanged(PhoneHost phone, PhoneState from, PhoneState to) {
        }

        /** The phone's computer was stopped (logout, battery empty); it cold-boots next time. */
        default void stopped(PhoneHost phone, String why) {
        }
    }

    /** Longest grace a listener can ask for before a sleep. */
    public static final long MAX_GRACE_MS = 2_000;

    private static final List<Listener> LISTENERS = new CopyOnWriteArrayList<>();

    private PhoneLifecycle() {}

    public static void addListener(Listener l) {
        LISTENERS.add(l);
    }

    public static void removeListener(Listener l) {
        LISTENERS.remove(l);
    }

    static long willSleep(PhoneHost phone, PhoneState next) {
        long grace = 0;
        for (Listener l : LISTENERS) grace = Math.max(grace, l.willSleep(phone, next));
        return Math.min(MAX_GRACE_MS, grace);
    }

    static void stateChanged(PhoneHost phone, PhoneState from, PhoneState to) {
        for (Listener l : LISTENERS) l.stateChanged(phone, from, to);
    }

    static void stopped(PhoneHost phone, String why) {
        for (Listener l : LISTENERS) l.stopped(phone, why);
    }
}
//?}
