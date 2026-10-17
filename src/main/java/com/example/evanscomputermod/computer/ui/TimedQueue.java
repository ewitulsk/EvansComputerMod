package com.example.evanscomputermod.computer.ui;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.List;

/**
 * The presentation queue of one canvas or pixel surface (UI protocol v2,
 * {@code CommitAt}; DISPLAY_PIPELINE_SPEC.md Part C4): frames an app
 * committed for a time, each a copy of the buffer as it will be, in
 * presentation order. The compositor shows each at its time (a pump timer),
 * so the server's frame always holds what is on screen now; the scene
 * exporter sends each one to clients as soon as it is queued.
 *
 * <p>Guarded by the compositor's monitor.
 */
final class TimedQueue {
    record Frame(long seq, long ptsUs, int[] argb) {}

    private final ArrayDeque<Frame> frames = new ArrayDeque<>();
    private long nextSeq = 1;
    /** The last frame the server shows (0 = none yet). */
    long appliedSeq;
    private long lastPts = Long.MIN_VALUE;
    /** {@code SurfaceSetContent} hint: 0 auto, 1 exact, 2 motion. */
    int hint;
    /** When the last frame was queued and shown (display clock µs), for "is this a timed stream". */
    long lastQueuedUs = Long.MIN_VALUE / 2, lastShownUs = Long.MIN_VALUE / 2;
    long queued, shown, dropped, flushed;

    int size() {
        return frames.size();
    }

    boolean isEmpty() {
        return frames.isEmpty();
    }

    /** The newest frame queued (what the next one builds on), or null. */
    int[] latest() {
        return frames.isEmpty() ? null : frames.peekLast().argb();
    }

    long latestSeq() {
        return frames.isEmpty() ? appliedSeq : frames.peekLast().seq();
    }

    long nextPts() {
        return frames.isEmpty() ? Long.MAX_VALUE : frames.peekFirst().ptsUs();
    }

    /** Queue {@code argb} (the queue keeps it) for {@code ptsUs}, after the frames already queued. */
    Frame add(long ptsUs, int[] argb, long nowUs) {
        long pts = Math.max(ptsUs, lastPts);
        lastPts = pts;
        Frame f = new Frame(nextSeq++, pts, argb);
        frames.addLast(f);
        queued++;
        lastQueuedUs = nowUs;
        return f;
    }

    /** The frames due at {@code nowUs}, oldest first; the caller shows the last. */
    List<Frame> takeDue(long nowUs) {
        List<Frame> due = new ArrayList<>();
        while (!frames.isEmpty() && frames.peekFirst().ptsUs() <= nowUs) due.add(frames.removeFirst());
        if (!due.isEmpty()) {
            appliedSeq = due.get(due.size() - 1).seq();
            shown++;
            dropped += due.size() - 1;
            lastShownUs = nowUs;
        }
        return due;
    }

    /** Take the oldest frame now (the queue is over its limit). */
    Frame takeOldest(long nowUs) {
        Frame f = frames.pollFirst();
        if (f != null) {
            appliedSeq = f.seq();
            shown++;
            lastShownUs = nowUs;
        }
        return f;
    }

    /** Drop the frames not shown yet. Returns how many; the frame after which they were dropped is {@link #appliedSeq}. */
    int flush() {
        int n = frames.size();
        frames.clear();
        lastPts = Long.MIN_VALUE;
        flushed += n;
        return n;
    }

    /** Frames were queued or shown within {@code windowUs}: a timed stream is running. */
    boolean active(long nowUs, long windowUs) {
        return !frames.isEmpty() || nowUs - Math.max(lastQueuedUs, lastShownUs) < windowUs;
    }
}
