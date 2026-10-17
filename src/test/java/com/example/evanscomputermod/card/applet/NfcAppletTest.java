package com.example.evanscomputermod.card.applet;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.card.Hex;
import org.junit.jupiter.api.Test;

import java.util.HexFormat;
import java.util.Random;

/** The sector lattice: Mix vectors (docs/cards/NFC_MIX.md), the 3-pass handshake, sectors and keys. */
public class NfcAppletTest {
    static final HexFormat HEX = HexFormat.of();

    public static final class Mem implements NfcApplet.Storage {
        public long uid;
        public NfcPublic pub;
        public NfcSecret secret;

        @Override public long uid() { return uid; }
        @Override public void ensureUid() { if (uid == 0) uid = 42; }
        @Override public NfcPublic nfcPublic() { return pub; }
        @Override public NfcSecret nfcSecret() { return secret; }
        @Override public void store(NfcPublic p, NfcSecret s) { pub = p; secret = s; }
    }

    @Test
    void mixVectors() {
        byte[] factory = NfcSecret.FACTORY_KEY;
        assertEquals("45434d2d4e46432d464143544f525930", Hex.of(factory));
        assertEquals("3e2e68d5726a6858461251213df31d59", Hex.of(NfcMix.mix(factory, new byte[16])));
        assertEquals("e8938dca0b939828c0b5e18c388b9702", Hex.of(NfcMix.mix(factory, HEX.parseHex("000102030405060708090a0b0c0d0e0f"))));
        assertEquals("f2668ac938caea225f16cf78a07cfe81", Hex.of(NfcMix.mix(HEX.parseHex("00112233445566778899aabbccddeeff"),
                HEX.parseHex("01020304050607081112131415161718"))));
        assertEquals("5d4fb41a84bab1a9fa2416dcc38b9978", Hex.of(NfcMix.mix(new byte[16], new byte[16])));
        byte[] ff = new byte[16];
        java.util.Arrays.fill(ff, (byte) 0xff);
        assertEquals("355b79459dc55581fc26ab3eb2eb6ce6", Hex.of(NfcMix.mix(ff, ff)));
    }

    @Test
    void handshakeReadWriteRekey() throws CardException {
        Mem m = new Mem();
        NfcApplet n = new NfcApplet(m);
        Random rnd = new Random(1);
        byte[] r1 = new byte[8];
        rnd.nextBytes(r1);
        NfcApplet.Challenge ch = n.challenge(1, r1, rnd);
        assertEquals(42, m.uid, "first contact gives the card its UID and factory sectors");
        assertTrue(m.secret.factoryKey(1));
        // a reader with the factory key
        assertArrayEquals(NfcApplet.m1(NfcSecret.FACTORY_KEY, r1, ch.r2()), ch.m1());
        assertTrue(n.checkM2(1, r1, ch.r2(), NfcApplet.m2(NfcSecret.FACTORY_KEY, r1, ch.r2())));
        // a reader with a wrong key
        byte[] wrong = new byte[16];
        assertFalse(n.checkM2(1, r1, ch.r2(), NfcApplet.m2(wrong, r1, ch.r2())));
        assertFalse(n.checkM2(1, r1, ch.r2(), new byte[15]));
        // data and rekey
        assertArrayEquals(new byte[64], n.read(1));
        byte[] data = new byte[64];
        java.util.Arrays.fill(data, (byte) 7);
        n.write(1, data);
        assertArrayEquals(data, n.read(1));
        assertTrue(n.info().written(1));
        assertFalse(n.info().written(2));
        byte[] key = HEX.parseHex("00112233445566778899aabbccddeeff");
        n.setKey(1, key);
        assertFalse(m.secret.factoryKey(1));
        NfcApplet.Challenge ch2 = n.challenge(1, r1, rnd);
        assertFalse(n.checkM2(1, r1, ch2.r2(), NfcApplet.m2(NfcSecret.FACTORY_KEY, r1, ch2.r2())), "the old key no longer opens it");
        assertTrue(n.checkM2(1, r1, ch2.r2(), NfcApplet.m2(key, r1, ch2.r2())));
        assertThrows(CardException.class, () -> n.read(8));
        assertThrows(CardException.class, () -> n.write(0, new byte[63]));
        assertThrows(CardException.class, () -> n.setKey(0, new byte[15]));
        assertThrows(CardException.class, () -> n.challenge(0, new byte[7], rnd));
    }
}
