package com.example.evanscomputermod.card;

import org.jetbrains.annotations.Nullable;

/**
 * What a reader remembers about the card in front of it: whether its PIN was verified
 * and which NFC sectors are open (plus the handshake in flight). The card itself keeps
 * no session; it all goes when the card leaves.
 */
public final class CardSession {
    public boolean pinOk;
    public int nfcOpen;
    public int pendingSector = -1;
    public byte @Nullable [] pendingR1, pendingR2;

    public void reset() {
        pinOk = false;
        nfcOpen = 0;
        pendingSector = -1;
        pendingR1 = pendingR2 = null;
    }

    public boolean sectorOpen(int sector) {
        return (nfcOpen >> sector & 1) != 0;
    }

    public void open(int sector) {
        nfcOpen |= 1 << sector;
    }
}
