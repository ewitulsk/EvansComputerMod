package com.example.evanscomputermod.computer.wasi;

/**
 * Thread of one WASI child process, with a {@link PauseGate}.
 *
 * <p><b>Pausing inside guest code on Chicory.</b> Chicory's interpreter and
 * its bytecode compiler both poll {@code Thread.currentThread().isInterrupted()}
 * at every wasm call and loop back-edge, and throw to cancel when it returns
 * true. That poll is a virtual call, so this subclass turns it into a pause
 * point: while the gate is paused and the thread is in guest code (not inside
 * a host call), it parks right there, with the wasm stack untouched, and
 * returns {@code false} on resume so the guest carries on. A kill still
 * interrupts the thread, which leaves the park and returns {@code true}, so
 * the runtime's normal cancellation runs.
 *
 * <p>Wasmtime runs guest code natively and never calls this method, so there
 * a busy loop only pauses at its next host call (see {@link PauseGate}).
 */
public final class PausableThread extends Thread {

    private final PauseGate gate;

    public PausableThread(Runnable task, String name, PauseGate gate) {
        super(task, name);
        this.gate = gate;
        gate.bind(this);
    }

    public PauseGate gate() {
        return gate;
    }

    @Override
    public boolean isInterrupted() {
        PauseGate g = gate;
        if (g != null && g.isPaused() &&Thread.currentThread() == this && g.mayParkInGuest()) {
            g.awaitResume();
        }
        return super.isInterrupted();
    }
}
