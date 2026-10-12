package com.example.evanscomputermod.computer;

/**
 * One client's place in a {@link DisplaySyncHub}: which frame it was last
 * sent, what it acknowledged, and when it is due a keyframe. The frames
 * themselves (and their diffs) belong to the hub and are shared by every
 * viewer, so this holds no shadow copy of the display.
 */
public class ClientSyncState {

    /** How often to send a full keyframe (ms). */
    public static final long KEYFRAME_INTERVAL_MS = 10_000;
    /** Max time to wait for a client ready ack before force-sending (ms). */
    public static final long READY_TIMEOUT_MS = 5_000;

    /** Hub generation of the last frame sent to this client; -1 before the first keyframe. */
    public long frameGen = -1;
    public long lastAckedGeneration = 0;
    public long lastSentGeneration = 0;
    public boolean needsKeyframe = true;  // true on first connect
    public long lastKeyframeTimeMs = 0;
    public long lastSentTimeMs = 0;

    public ClientSyncState() {
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
        if (needsKeyframe || frameGen < 0) return true;
        return System.currentTimeMillis() - lastKeyframeTimeMs > KEYFRAME_INTERVAL_MS;
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

    /** Handle client acknowledgment. */
    public void onClientReady(long ackedGeneration) {
        if (ackedGeneration > lastAckedGeneration) {
            lastAckedGeneration = ackedGeneration;
        }
    }
}
