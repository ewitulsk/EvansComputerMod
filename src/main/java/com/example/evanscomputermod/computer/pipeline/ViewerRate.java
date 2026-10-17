package com.example.evanscomputermod.computer.pipeline;

import java.util.ArrayDeque;

/**
 * One viewer's link as the server sees it from acks (DISPLAY_PIPELINE_SPEC.md
 * Part B2): round-trip time, jitter (RFC 3550 style, from the client's receive
 * times against the server's send times) and the throughput the client
 * actually received over the last second. The hubs keep one per viewer;
 * lossy encoding picks its quality from them and {@code /ecm gui stats}
 * reports them.
 *
 * <p>Not thread-safe; the owning hub's monitor guards it.
 */
public final class ViewerRate {
    /** How far back throughput is measured. */
    static final long WINDOW_US = 1_000_000L;
    private static final int MAX_PENDING = 256;

    private record Sent(long gen, long sendUs, long bytes) {}

    private record Acked(long atUs, long bytes) {}

    private final ArrayDeque<Sent> pending = new ArrayDeque<>();
    private final ArrayDeque<Acked> acked = new ArrayDeque<>();
    private long ackedBytesInWindow;
    private double rttUs = -1, jitterUs;
    private long lastTransitUs;
    private boolean haveTransit;
    private long samples;

    /** A frame of {@code bytes} for generation {@code gen} went out at {@code sendUs} (display clock). */
    public void sent(long gen, long bytes, long sendUs) {
        pending.addLast(new Sent(gen, sendUs, bytes));
        while (pending.size() > MAX_PENDING) pending.removeFirst();
    }

    /**
     * The client acked up to {@code gen}, which it received at its own time
     * {@code clientRecvUs} (0 = unknown, an old client), and the ack reached
     * the server at {@code nowUs}.
     */
    public void acked(long gen, long clientRecvUs, long nowUs) {
        Sent last = null;
        long bytes = 0;
        while (!pending.isEmpty() && pending.peekFirst().gen <= gen) {
            last = pending.removeFirst();
            bytes += last.bytes;
        }
        if (last == null) return;
        double rtt = Math.max(0, nowUs - last.sendUs);
        rttUs = rttUs < 0 ? rtt : rttUs + (rtt - rttUs) / 8.0;
        if (clientRecvUs > 0) {
            long transit = clientRecvUs - last.sendUs;
            if (haveTransit) {
                double d = Math.abs(transit - lastTransitUs);
                jitterUs += (d - jitterUs) / 16.0;
            }
            lastTransitUs = transit;
            haveTransit = true;
        }
        samples++;
        acked.addLast(new Acked(nowUs, bytes));
        ackedBytesInWindow += bytes;
        trim(nowUs);
    }

    private void trim(long nowUs) {
        while (!acked.isEmpty() && nowUs - acked.peekFirst().atUs > WINDOW_US) {
            ackedBytesInWindow -= acked.removeFirst().bytes;
        }
    }

    /** Forget frames in flight (the viewer is restarted with a keyframe). */
    public void reset() {
        pending.clear();
    }

    /** Smoothed round trip in ms, or -1 before the first ack. */
    public double rttMs() {
        return rttUs < 0 ? -1 : rttUs / 1000.0;
    }

    /** Smoothed jitter in ms. */
    public double jitterMs() {
        return jitterUs / 1000.0;
    }

    /** What the client received over the last second, in kbit/s. */
    public double deliveredKbps(long nowUs) {
        trim(nowUs);
        return ackedBytesInWindow * 8.0 / 1000.0 / (WINDOW_US / 1_000_000.0);
    }

    /** Bytes sent and not yet acked. */
    public long inFlightBytes() {
        long n = 0;
        for (Sent s : pending) n += s.bytes;
        return n;
    }

    public long samples() {
        return samples;
    }

    /** A copy of the numbers for reports: {rtt ms, jitter ms, kbps}. */
    public double[] snapshot(long nowUs) {
        return new double[]{rttMs(), jitterMs(), deliveredKbps(nowUs)};
    }
}
