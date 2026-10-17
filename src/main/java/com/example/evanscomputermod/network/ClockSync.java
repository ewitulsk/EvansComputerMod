package com.example.evanscomputermod.network;

import java.util.ArrayDeque;

/**
 * Client side: how the server's display clock relates to this client's
 * (DISPLAY_PIPELINE_SPEC.md Part C1), one estimate per server connection.
 * Every timed message carries the server's send time; the client notes when
 * it arrived. {@code send - recv} is the clock offset minus the network
 * delay of that message, so its maximum over recent messages (the least
 * delayed one, as NTP's minimum filter) is the offset estimate:
 * {@code serverTime - offset} is the same instant on this client's clock.
 *
 * <p>Thread-safe (the decode thread notes, the render thread reads).
 */
public final class ClockSync {
    /** Samples older than this are forgotten (µs), so a changed route is followed. */
    static final long WINDOW_US = 10_000_000;

    private record Sample(long recvUs, long offsetUs) {}

    /** Sliding-window maximum: offsets decrease from head to tail. */
    private final ArrayDeque<Sample> max = new ArrayDeque<>();
    private long offsetUs;
    private boolean known;
    private long samples;

    /** The connection's estimate (one per client). */
    public static final ClockSync INSTANCE = new ClockSync();

    /** This client's clock, in µs. */
    public static long clientNowUs() {
        return System.nanoTime() / 1000L;
    }

    /** A message sent at server time {@code serverSendUs} arrived at client time {@code clientRecvUs}. */
    public synchronized void note(long serverSendUs, long clientRecvUs) {
        long o = serverSendUs - clientRecvUs;
        while (!max.isEmpty() && max.peekLast().offsetUs <= o) max.removeLast();
        max.addLast(new Sample(clientRecvUs, o));
        while (!max.isEmpty() && clientRecvUs - max.peekFirst().recvUs > WINDOW_US) max.removeFirst();
        offsetUs = max.peekFirst().offsetUs;
        known = true;
        samples++;
    }

    /** Server time minus client time, µs (0 before the first timed message). */
    public synchronized long offsetUs() {
        return offsetUs;
    }

    public synchronized boolean known() {
        return known;
    }

    /** A server display-clock time on this client's clock. */
    public synchronized long toClient(long serverUs) {
        return serverUs - offsetUs;
    }

    /**
     * How much longer than the quickest recent message this one took (µs):
     * its queueing delay, what the playout buffer must absorb.
     */
    public synchronized long excessDelayUs(long serverSendUs, long clientRecvUs) {
        return Math.max(0, offsetUs - (serverSendUs - clientRecvUs));
    }

    public synchronized long samples() {
        return samples;
    }

    /** A new connection: forget everything. */
    public synchronized void reset() {
        max.clear();
        offsetUs = 0;
        known = false;
        samples = 0;
    }
}
