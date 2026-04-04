package com.example.evanscomputermod.computer;

import java.util.zip.Deflater;

/**
 * Per-client synchronization state for the delta sync protocol.
 * Each connected player tracking a terminal gets their own sync state.
 */
public class ClientSyncState {

    /** How often to send a full keyframe (ms). */
    public static final long KEYFRAME_INTERVAL_MS = 10_000;
    /** Max time to wait for a client ready ack before force-sending (ms). */
    public static final long READY_TIMEOUT_MS = 5_000;

    public final FramebufferDiffTracker tracker = new FramebufferDiffTracker();
    public long lastAckedGeneration = 0;
    public long lastSentGeneration = 0;
    public boolean needsKeyframe = true;  // true on first connect
    public long lastKeyframeTimeMs = 0;
    public long lastSentTimeMs = 0;

    /** Reusable Deflater for zlib compression (call reset() between uses). */
    public final Deflater deflater = new Deflater(Deflater.BEST_SPEED);

    public ClientSyncState() {
    }

    /** Initialize the tracker from the current display state. */
    public void init(TerminalDisplay display) {
        tracker.initShadow(display);
    }

    /** Whether the client has acknowledged the last sent frame. */
    public boolean isClientReady() {
        if (lastSentGeneration == 0) return true; // nothing sent yet
        if (lastAckedGeneration >= lastSentGeneration) return true;
        // Force send if client hasn't acked in READY_TIMEOUT_MS
        return System.currentTimeMillis() - lastSentTimeMs > READY_TIMEOUT_MS;
    }

    /** Whether we should send a keyframe instead of a delta. */
    public boolean shouldSendKeyframe() {
        if (needsKeyframe) return true;
        return System.currentTimeMillis() - lastKeyframeTimeMs > KEYFRAME_INTERVAL_MS;
    }

    /** Record that a packet was sent. */
    public void markSent(long generation) {
        lastSentGeneration = generation;
        lastSentTimeMs = System.currentTimeMillis();
    }

    /** Record that a keyframe was sent. */
    public void markKeyframeSent(long generation) {
        markSent(generation);
        needsKeyframe = false;
        lastKeyframeTimeMs = System.currentTimeMillis();
    }

    /** Handle client acknowledgment. */
    public void onClientReady(long ackedGeneration) {
        if (ackedGeneration > lastAckedGeneration) {
            lastAckedGeneration = ackedGeneration;
        }
    }

    /** Clean up resources. */
    public void close() {
        deflater.end();
    }
}
