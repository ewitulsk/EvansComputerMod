package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.computer.pipeline.ViewerRate;

import java.util.ArrayDeque;

/**
 * One client's place in a {@link DisplaySyncHub}: which frame it was last
 * sent, which frames are still unacknowledged, and when it is due a keyframe.
 * The frames themselves (and their diffs) belong to the hub and are shared by
 * every viewer, so this holds no shadow copy of the display.
 *
 * <p>Flow control is a window (DISPLAY_PIPELINE_SPEC.md Part B1): at most
 * {@code windowFrames} frames and {@code windowBytes} bytes may be
 * unacknowledged. When either limit is hit, changes coalesce into the next
 * frame. A frame that stays unacknowledged for {@link #READY_TIMEOUT_MS}
 * (a lost ack, a stalled client) restarts the viewer with a keyframe rather
 * than resending a stale chain, so a screen can't freeze for more than about
 * a second. The hub's monitor guards it.
 */
public class ClientSyncState {

    /** How often to send a full keyframe (ms). */
    public static final long KEYFRAME_INTERVAL_MS = 10_000;
    /** A frame unacked this long gets the viewer a keyframe (ms). */
    public static final long READY_TIMEOUT_MS = 1_000;

    /** Hub generation of the last frame sent to this client; -1 before the first keyframe. */
    public long frameGen = -1;
    public long lastAckedGeneration = 0;
    public long lastSentGeneration = 0;
    public boolean needsKeyframe = true;  // true on first connect
    public long lastKeyframeTimeMs = 0;
    public long lastSentTimeMs = 0;
    /** Frames sent and not acked yet: {generation, bytes, sent at ms}. */
    private final ArrayDeque<long[]> inFlight = new ArrayDeque<>();
    private long inFlightBytes;
    /** What the acks say about this viewer's link (RTT, jitter, throughput). */
    public final ViewerRate rate = new ViewerRate();
    /** Timeouts so far (diagnostics). */
    public long timeouts;

    public ClientSyncState() {
    }

    /** Whether the client acknowledged everything sent (the old one-frame rule; tests). */
    public boolean isClientReady() {
        return inFlight.isEmpty();
    }

    /** May another frame go out within the window? */
    public boolean canSend(int windowFrames, long windowBytes) {
        return inFlight.size() < Math.max(1, windowFrames) && inFlightBytes < Math.max(1, windowBytes);
    }

    /** Frames and bytes unacknowledged now. */
    public int inFlightFrames() {
        return inFlight.size();
    }

    public long inFlightBytes() {
        return inFlightBytes;
    }

    /**
     * The oldest unacked frame has waited longer than {@link #READY_TIMEOUT_MS}:
     * the viewer is restarted with a keyframe (and its window emptied).
     * Returns true if it timed out.
     */
    public boolean checkTimeout(long nowMs) {
        long[] oldest = inFlight.peekFirst();
        if (oldest == null || nowMs - oldest[2] <= READY_TIMEOUT_MS) return false;
        inFlight.clear();
        inFlightBytes = 0;
        rate.reset();
        needsKeyframe = true;
        timeouts++;
        return true;
    }

    /** Whether we should send a keyframe instead of a delta. */
    public boolean shouldSendKeyframe() {
        if (needsKeyframe || frameGen < 0) return true;
        return System.currentTimeMillis() - lastKeyframeTimeMs > KEYFRAME_INTERVAL_MS;
    }

    /** Record one frame of {@code bytes} for {@code generation} going out. */
    public void sent(long generation, long bytes, long nowMs, long nowUs) {
        inFlight.addLast(new long[]{generation, bytes, nowMs});
        inFlightBytes += bytes;
        rate.sent(generation, bytes, nowUs);
    }

    /** Record that frames up to {@code generation} were sent. */
    public void markSent(long generation) {
        frameGen = generation;
        lastSentGeneration = generation;
        lastSentTimeMs = System.currentTimeMillis();
    }

    /** Record that a keyframe was sent. */
    public void markKeyframeSent(long generation) {
        markSent(generation);
        needsKeyframe = false;
        lastKeyframeTimeMs = System.currentTimeMillis();
    }

    /** Handle client acknowledgment (receive time unknown). */
    public void onClientReady(long ackedGeneration) {
        onClientReady(ackedGeneration, 0, com.example.evanscomputermod.computer.pipeline.DisplayClock.nowUs());
    }

    /**
     * The client got everything up to {@code ackedGeneration}; it received the
     * last of it at its own time {@code clientRecvUs} (0 = unknown).
     */
    public void onClientReady(long ackedGeneration, long clientRecvUs, long nowUs) {
        if (ackedGeneration > lastAckedGeneration) {
            lastAckedGeneration = ackedGeneration;
        }
        while (!inFlight.isEmpty() && inFlight.peekFirst()[0] <= ackedGeneration) {
            inFlightBytes -= inFlight.removeFirst()[1];
        }
        rate.acked(ackedGeneration, clientRecvUs, nowUs);
    }
}
