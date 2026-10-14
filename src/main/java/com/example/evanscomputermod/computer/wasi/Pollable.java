package com.example.evanscomputermod.computer.wasi;

/**
 * A descriptor that {@code poll_oneoff} and {@code sock_poll} can wait on
 * locally (the UI fd, §3 "One event loop"): readiness is known on the Java
 * side, and the descriptor notifies registered waiters (with
 * {@code notifyAll} on the waiter object) when it changes.
 */
public interface Pollable {
    /** Data to read, or the other end is gone (a read then reports it). */
    boolean readReady();

    /** Bytes ready to read now. */
    int readable();

    /** The other end is gone. */
    boolean hungUp();

    void addWaiter(Object waiter);

    void removeWaiter(Object waiter);
}
