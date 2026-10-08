package com.example.evanscomputermod.radio.medium;

import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;

import java.util.UUID;

/**
 * A registered endpoint inside {@link WorldRadioMedium}: a dense index for pair
 * keys plus the bookkeeping the server tick keeps about it. Volatile fields are
 * read by transmitting threads; the rest belong to the server thread.
 */
public final class Node {
    final RadioEndpoint ep;
    final UUID id;
    final int idx;

    volatile boolean removed;
    volatile boolean invalidated;
    /** Last transmit frequency and band shard (so pairs of transmit-only radios get a band). */
    volatile double lastTxHz;
    volatile int lastTxBand = -1;

    // Spatial index membership (written under the medium's index lock or by the index build).
    volatile int indexBand = -1;
    volatile Channel indexedChannel;
    volatile int indexedCellX, indexedCellZ;

    // Server-thread state.
    Pose computedPose;
    long lastMoveTick = Long.MIN_VALUE / 2;
    boolean discover = true;
    boolean movePending;

    Node(RadioEndpoint ep, int idx) {
        this.ep = ep;
        this.id = ep.id();
        this.idx = idx;
    }

    public RadioEndpoint endpoint() {
        return ep;
    }

    public int index() {
        return idx;
    }
}
