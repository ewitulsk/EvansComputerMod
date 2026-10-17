package com.example.evanscomputermod.card;

import net.minecraft.util.StringRepresentable;

import java.util.Locale;

/**
 * The chassis of a card (docs/cards/CARDS.md, "The card item"): its face size in texels,
 * the fixed features the renderer draws over the painted faces (a chip contact pad, the
 * magnetic stripe) and which applets it carries. Each kind is its own item
 * ({@code bank_card}, {@code nfc_card}, {@code hybrid_card}, {@code sim_card}).
 */
public enum CardKind implements StringRepresentable {
    BANK(32, 20, true, true, true, false, false),
    NFC(32, 20, false, false, false, true, false),
    HYBRID(32, 20, true, true, true, true, false),
    SIM(8, 13, true, false, false, false, true);

    public final int faceWidth, faceHeight;
    /** Chip contact pad on the front (bank, hybrid, sim). */
    public final boolean chipPad;
    /** Magnetic stripe on the back (bank, hybrid): three plain-text tracks. */
    public final boolean stripe;
    public final boolean bank, nfc, sim;

    CardKind(int w, int h, boolean chipPad, boolean stripe, boolean bank, boolean nfc, boolean sim) {
        this.faceWidth = w;
        this.faceHeight = h;
        this.chipPad = chipPad;
        this.stripe = stripe;
        this.bank = bank;
        this.nfc = nfc;
        this.sim = sim;
    }

    public int texels() {
        return faceWidth * faceHeight;
    }

    /** The masked texels of a face (true = fixed feature, never painted): {@code [y * w + x]}. */
    public boolean[] mask(boolean front) {
        boolean[] m = new boolean[texels()];
        if (front && chipPad) {
            // a 6x5 contact pad: at the left, vertically centred (a SIM: near the top, centred)
            int pw = 6, ph = 5;
            int x0 = this == SIM ? 1 : 3, y0 = this == SIM ? 3 : (faceHeight - ph) / 2;
            for (int y = y0; y < y0 + ph; y++) for (int x = x0; x < x0 + pw; x++) m[y * faceWidth + x] = true;
        }
        if (!front && stripe) {
            // rows 2..5 full width
            for (int y = 2; y <= 5; y++) for (int x = 0; x < faceWidth; x++) m[y * faceWidth + x] = true;
        }
        return m;
    }

    public String id() {
        return name().toLowerCase(Locale.ROOT);
    }

    @Override
    public String getSerializedName() {
        return id();
    }

    public static CardKind of(String id) {
        for (CardKind k : values()) if (k.id().equals(id)) return k;
        throw new IllegalArgumentException("no card kind " + id);
    }
}
