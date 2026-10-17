package com.example.evanscomputermod.card.nfc;

import com.example.evanscomputermod.card.applet.NfcApplet;
import org.jetbrains.annotations.Nullable;

/**
 * What the air interface needs of a card: its NFC applet (null for a card without one),
 * its UID and its kinds bitmask ({@link NfcAir#KIND_BANK} …). An item stack provides
 * one through {@code CardEndpointTracker}; tests provide one over an in-memory applet.
 */
public interface NfcCard {
    @Nullable NfcApplet nfc();

    long uid();

    int kinds();

    /** The same physical card as {@code other} (sessions survive a refresh of the stack reference). */
    default boolean sameAs(NfcCard other) {
        return other != null && other.uid() == uid() && uid() != 0;
    }
}
