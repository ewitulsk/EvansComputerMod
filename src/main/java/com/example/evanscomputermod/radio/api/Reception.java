package com.example.evanscomputermod.radio.api;

import java.util.UUID;

/**
 * A frame as one receiver heard it, with radiotap-style metadata.
 *
 * @param rssiDbm   received signal strength
 * @param sinrDb    signal to noise-plus-interference at the receiver
 * @param timestampMicros end of the frame on the world airtime clock
 */
public record Reception(UUID from, Emission emission, double rssiDbm, double sinrDb, long timestampMicros) {

    public byte[] payload() {
        return emission.payload();
    }
}
