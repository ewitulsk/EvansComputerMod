package com.example.evanscomputermod.computer.display;

import com.example.evanscomputermod.computer.TerminalDisplay;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.List;

/**
 * A display's presentation queue (DISPLAY_PIPELINE_SPEC.md Part C4): frames a
 * program queued with {@code gfx_present_at}, each a copy of the back buffer
 * plus the display-clock time it should be seen at, in presentation order.
 *
 * <p>Two parties read it. The {@link DisplayDevice} that owns it applies each
 * frame to its scanout at the frame's time (a pump timer), so the server's
 * {@link TerminalDisplay} always holds "what is on screen now" for headless
 * computers, screenshots and late joiners. Every sync hub streaming the
 * scanout reads the frames as soon as they are queued, with its own cursor
 * ({@link Frame#seq}), and sends them to clients tagged with their time, so
 * the clients' playout buffers fill ahead.
 *
 * <p>Sequence numbers only grow. {@link #appliedSeq()} is the last frame the
 * scanout shows; {@link #flushEpoch()} counts flushes, after which hubs drop
 * what they sent ahead. Its own lock is always taken last (after the device's
 * and the display's monitors).
 */
public final class PresentQueue {
    /** One queued frame. The arrays are the queue's own copies, never changed after queueing. */
    public record Frame(long seq, long ptsUs, int mode, int width, int height, int format, int[] palette, byte[] pixels) {
        /** Same shape as the display (deltas carry no sizes)? */
        public boolean fits(TerminalDisplay d) {
            return d.getDisplayMode() == mode && d.getGfxWidth() == width && d.getGfxHeight() == height
                    && d.getPixelFormat() == format;
        }
    }

    /** What a hub needs in one consistent read. */
    public record State(long appliedSeq, int flushEpoch, long flushedAfterSeq, List<Frame> pending, TerminalDisplay target) {}

    private final ArrayDeque<Frame> queued = new ArrayDeque<>();
    private long nextSeq = 1;
    private long appliedSeq;
    private int flushEpoch;
    /** The last flush dropped everything after this frame. */
    private long flushedAfterSeq;
    private long lastPtsUs = Long.MIN_VALUE;
    /** The scanout the frames are for (a hub only uses a queue that feeds its display). */
    private TerminalDisplay target;
    /** Arrays of frames already shown, for the next copies (A7: no new array per frame). */
    private final ArrayDeque<byte[]> spare = new ArrayDeque<>();

    // Counters (gfx_queue_state, /ecm gui stats, scenarios).
    private long framesQueued, framesShown, framesLate, framesFlushed, lateUsTotal;

    public synchronized int size() {
        return queued.size();
    }

    /** How far the last queued frame is ahead of {@code nowUs}, in ms (0 when empty). */
    public synchronized int aheadMs(long nowUs) {
        if (queued.isEmpty()) return 0;
        return (int) Math.max(0, (queued.peekLast().ptsUs() - nowUs) / 1000);
    }

    public synchronized boolean isEmpty() {
        return queued.isEmpty();
    }

    /** The earliest queued frame's time, or Long.MAX_VALUE. */
    public synchronized long nextPtsUs() {
        return queued.isEmpty() ? Long.MAX_VALUE : queued.peekFirst().ptsUs();
    }

    /**
     * Queue a copy of {@code pixels} (and {@code palette}, may be null) for
     * {@code ptsUs}. A time before the last queued frame's is moved up to it
     * (frames are shown in the order they were queued). Returns the frame.
     */
    public synchronized Frame add(long ptsUs, int mode, int w, int h, int format, int[] palette, byte[] pixels, TerminalDisplay target) {
        long pts = Math.max(ptsUs, lastPtsUs);
        lastPtsUs = pts;
        byte[] copy = spareArray(pixels.length);
        System.arraycopy(pixels, 0, copy, 0, pixels.length);
        Frame f = new Frame(nextSeq++, pts, mode, w, h, format, palette == null ? null : palette.clone(), copy);
        queued.addLast(f);
        this.target = target;
        framesQueued++;
        return f;
    }

    private byte[] spareArray(int n) {
        while (!spare.isEmpty()) {
            byte[] b = spare.pollFirst();
            if (b.length == n) return b;
        }
        return new byte[n];
    }

    /**
     * Take the frames due at {@code nowUs} (pts reached): the scanout shows the
     * last of them; the others were never on screen. Returns them in order
     * (empty if none is due). The caller applies the last one and then calls
     * {@link #markApplied}.
     */
    public synchronized List<Frame> takeDue(long nowUs) {
        List<Frame> due = new ArrayList<>();
        while (!queued.isEmpty() && queued.peekFirst().ptsUs() <= nowUs) due.add(queued.removeFirst());
        return due;
    }

    /** The scanout now shows {@code f}, {@code lateUs} after its time. */
    public synchronized void markApplied(Frame f, long lateUs, int skipped) {
        appliedSeq = Math.max(appliedSeq, f.seq());
        framesShown++;
        framesFlushed += skipped;
        if (lateUs > 20_000) {
            framesLate++;
            lateUsTotal += lateUs;
        }
    }

    /** A frame's pixel array can be reused (hubs read frames under the display's monitor before this). */
    public synchronized void recycle(Frame f) {
        if (spare.size() < 4) spare.addLast(f.pixels());
    }

    /** Drop every frame not shown yet (seek, pause, stop). Returns how many. */
    public synchronized int flush() {
        int n = queued.size();
        for (Frame f : queued) if (spare.size() < 4) spare.addLast(f.pixels());
        queued.clear();
        flushEpoch++;
        flushedAfterSeq = appliedSeq;
        // Frames queued after a flush start from "now" again.
        lastPtsUs = Long.MIN_VALUE;
        framesFlushed += n;
        return n;
    }

    public synchronized State state() {
        return new State(appliedSeq, flushEpoch, flushedAfterSeq, List.copyOf(queued), target);
    }

    public synchronized long appliedSeq() {
        return appliedSeq;
    }

    public synchronized int flushEpoch() {
        return flushEpoch;
    }

    public synchronized long framesQueued() {
        return framesQueued;
    }

    public synchronized long framesShown() {
        return framesShown;
    }

    public synchronized long framesLate() {
        return framesLate;
    }

    public synchronized long framesFlushed() {
        return framesFlushed;
    }

    /** Mean lateness of late frames, ms. */
    public synchronized double meanLateMs() {
        return framesLate == 0 ? 0 : lateUsTotal / 1000.0 / framesLate;
    }
}
