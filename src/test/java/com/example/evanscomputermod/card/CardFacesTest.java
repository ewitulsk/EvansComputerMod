package com.example.evanscomputermod.card;

import static org.junit.jupiter.api.Assertions.*;

import com.mojang.serialization.JsonOps;
import org.junit.jupiter.api.Test;

import java.util.Random;

/** Painted faces, their packing and codecs; the kinds' masks; stripes and their corruption. */
public class CardFacesTest {

    @Test
    void packingAndCodec() {
        CardFaces f = CardFaces.blank(CardKind.BANK);
        assertTrue(f.fits(CardKind.BANK));
        assertFalse(f.fits(CardKind.SIM));
        assertEquals(320, f.front().length);
        byte[] front = f.front().clone();
        CardFaces.setTexel(front, 0, 5);
        CardFaces.setTexel(front, 1, 0xA);
        CardFaces.setTexel(front, 639, 3);
        assertEquals(5, CardFaces.texel(front, 0));
        assertEquals(0xA, CardFaces.texel(front, 1));
        assertEquals(3, CardFaces.texel(front, 639));
        assertEquals(0, CardFaces.texel(front, 640), "past the end reads 0");
        CardFaces g = new CardFaces(new int[] {0xFF0000, 0x00FF00, 0, 0, 0, 0xFFFFFF}, front, f.back());
        assertEquals(0xFFFFFFFF, g.argb(true, 0), "index past the palette clamps to the last colour");
        assertEquals(0xFFFF0000, g.argb(false, 0));
        var json = CardFaces.CODEC.encodeStart(JsonOps.INSTANCE, g).getOrThrow();
        CardFaces back = CardFaces.CODEC.parse(JsonOps.INSTANCE, json).getOrThrow();
        assertEquals(g, back);
        assertEquals(g.contentHash(), back.contentHash());
        assertNotEquals(g.contentHash(), f.contentHash());
        var buf = new net.minecraft.network.RegistryFriendlyByteBuf(io.netty.buffer.Unpooled.buffer(), net.minecraft.core.RegistryAccess.EMPTY);
        CardFaces.STREAM_CODEC.encode(buf, g);
        assertEquals(g, CardFaces.STREAM_CODEC.decode(buf));
        assertThrows(IllegalArgumentException.class, () -> new CardFaces(new int[17], front, front));
        assertThrows(IllegalArgumentException.class, () -> new CardFaces(new int[0], front, front));
    }

    @Test
    void masks() {
        boolean[] bankFront = CardKind.BANK.mask(true), bankBack = CardKind.BANK.mask(false);
        assertTrue(bankFront[7 * 32 + 3], "chip pad");
        assertFalse(bankFront[0]);
        assertTrue(bankBack[2 * 32], "stripe rows 2-5");
        assertTrue(bankBack[5 * 32 + 31]);
        assertFalse(bankBack[6 * 32]);
        assertFalse(bankBack[1 * 32]);
        boolean[] nfcBack = CardKind.NFC.mask(false);
        for (boolean b : nfcBack) assertFalse(b, "an NFC card has no stripe");
        for (boolean b : CardKind.NFC.mask(true)) assertFalse(b, "nor a pad");
        assertTrue(CardKind.HYBRID.mask(false)[3 * 32]);
        assertTrue(CardKind.SIM.mask(true)[3 * 8 + 1]);
        assertEquals(8 * 13, CardKind.SIM.texels());
    }

    @Test
    void stripes() {
        Stripe s = new Stripe("ALICE", "4000123412341234", "", false);
        assertFalse(s.corrupted());
        assertThrows(IllegalArgumentException.class, () -> new Stripe("x".repeat(65), "", "", false));
        assertThrows(IllegalArgumentException.class, () -> new Stripe("", "x".repeat(33), "", false));
        assertThrows(IllegalArgumentException.class, () -> new Stripe("", "", "x".repeat(97), false));
        assertThrows(IllegalArgumentException.class, () -> new Stripe("tab\there", "", "", false));
        Stripe c = s.corrupt(new Random(3));
        assertTrue(c.corrupted());
        assertNotEquals(s.t1() + s.t2(), c.t1() + c.t2(), "corruption changes the text");
        assertEquals(s.t1().length(), c.t1().length(), "garbage, not blanks");
        assertEquals("", c.t3());
        var json = Stripe.CODEC.encodeStart(JsonOps.INSTANCE, c).getOrThrow();
        assertEquals(c, Stripe.CODEC.parse(JsonOps.INSTANCE, json).getOrThrow());
    }
}
