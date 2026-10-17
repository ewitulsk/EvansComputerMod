package com.example.evanscomputermod.card.applet;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.card.Hex;
import org.junit.jupiter.api.Test;

import java.util.HexFormat;

/**
 * The bank chip: TAG and PIN-hash vectors (docs/cards/NFC_MIX.md, from scripts/cards-vectors.py),
 * the counter, the PIN session and the lock, over a test storage.
 */
public class BankAppletTest {
    static final HexFormat HEX = HexFormat.of();
    static final byte[] K1 = HEX.parseHex("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");

    /** An in-memory card. */
    static final class Mem implements BankApplet.Storage {
        long uid;
        BankPublic pub;
        BankSecret secret;
        int count = 1;

        @Override public long uid() { return uid; }
        @Override public void ensureUid() { if (uid == 0) uid = 0x0102030405060708L; }
        @Override public BankPublic bankPublic() { return pub; }
        @Override public BankSecret bankSecret() { return secret; }
        @Override public int count() { return count; }
        @Override public void store(BankPublic p, BankSecret s) { pub = p; if (s != null) secret = s; }
    }

    @Test
    void tagVectors() {
        assertEquals("aef6052d8232ad7c4db312596726e55a", Hex.of(BankApplet.tag(K1, HEX.parseHex("0001020304050607"), 1, 0)));
        assertEquals("ce2e7e5ad8d1e0b8bd42aa2b8903e5b7", Hex.of(BankApplet.tag(K1, HEX.parseHex("0001020304050607"), 2, 1500)));
        assertEquals("c1e9e0459b7a23ecb6d54dc7772a25af", Hex.of(BankApplet.tag(new byte[32], new byte[8], 0, 0)));
        byte[] ff32 = new byte[32], ff8 = new byte[8];
        java.util.Arrays.fill(ff32, (byte) 0xff);
        java.util.Arrays.fill(ff8, (byte) 0xff);
        assertEquals("6987930a133eede5b3b6de9ef58f0e91", Hex.of(BankApplet.tag(ff32, ff8, Long.MAX_VALUE, 1L << 40)));
    }

    @Test
    void pinHashVectors() {
        assertEquals("282fbfbe3f85ce96542b0435cc442d28745c372aaacf04b6229e657998169fd2", Hex.of(BankApplet.pinHash(0x0102030405060708L, "1234")));
        assertEquals("72a0521796bf77ccf576d8f5554d61b5a5cd371fad7492f3b64514b7e2a4cb19", Hex.of(BankApplet.pinHash(0, "00000000")));
    }

    @Test
    void writeSignAndCount() throws CardException {
        Mem m = new Mem();
        BankApplet b = new BankApplet(m);
        assertTrue(b.isBlank());
        assertThrows(CardException.class, () -> b.sign(new byte[8], 0, false));
        b.write("bank-red", "4000123412341234", K1, null, false);
        assertFalse(b.isBlank());
        assertEquals(0x0102030405060708L, m.uid);
        assertThrows(CardException.class, () -> b.write("bank-red", "4000123412341234", K1, null, false), "write once");
        BankApplet.Signature s1 = b.sign(HEX.parseHex("0001020304050607"), 0, false);
        assertEquals(1, s1.ctr());
        assertEquals("aef6052d8232ad7c4db312596726e55a", s1.tagHex());
        BankApplet.Signature s2 = b.sign(HEX.parseHex("0001020304050607"), 1500, false);
        assertEquals(2, s2.ctr());
        assertEquals("ce2e7e5ad8d1e0b8bd42aa2b8903e5b7", s2.tagHex());
        assertEquals(2, m.pub.ctr());
        assertThrows(CardException.class, () -> b.sign(new byte[7], 0, false));
        assertThrows(CardException.class, () -> b.sign(new byte[8], -1, false));
    }

    @Test
    void pinSessionAndLock() throws CardException {
        Mem m = new Mem();
        BankApplet b = new BankApplet(m);
        assertThrows(CardException.class, () -> b.write("bank-red", "4000123412341234", K1, "12", true), "a short PIN");
        assertThrows(CardException.class, () -> b.write("bank-red", "4000123412341234", K1, null, true), "pin_required needs a PIN");
        b.write("bank-red", "4000123412341234", K1, "1234", true);
        assertThrows(CardException.class, () -> b.sign(new byte[8], 0, false), "PIN required");
        assertFalse(b.verifyPin("0000"));
        assertEquals(2, b.triesLeft());
        assertTrue(b.verifyPin("1234"));
        assertEquals(3, b.triesLeft(), "a good PIN resets the count");
        assertEquals(1, b.sign(new byte[8], 10, true).ctr());
        assertFalse(b.verifyPin("0001"));
        assertFalse(b.verifyPin("0002"));
        assertFalse(b.verifyPin("0003"));
        assertTrue(m.pub.locked());
        assertEquals(0, b.triesLeft());
        assertThrows(CardException.class, () -> b.verifyPin("1234"), "locked");
        assertThrows(CardException.class, () -> b.sign(new byte[8], 0, true), "locked");
    }

    @Test
    void secretsAreNotInTheClientEncoding() {
        var type = BankSecret.componentType();
        BankSecret s = BankSecret.of(K1, BankApplet.pinHash(1, "1234"));
        var buf = new net.minecraft.network.RegistryFriendlyByteBuf(io.netty.buffer.Unpooled.buffer(), net.minecraft.core.RegistryAccess.EMPTY);
        type.streamCodec().encode(buf, s);
        assertEquals(0, buf.readableBytes());
        assertSame(BankSecret.HIDDEN, type.streamCodec().decode(new net.minecraft.network.RegistryFriendlyByteBuf(io.netty.buffer.Unpooled.buffer(), net.minecraft.core.RegistryAccess.EMPTY)));
        assertFalse(s.toString().contains(Hex.of(K1)));
        var saved = type.codec().encodeStart(com.mojang.serialization.JsonOps.INSTANCE, s).getOrThrow();
        assertTrue(saved.toString().contains(Hex.of(K1)));
        assertEquals(s, type.codec().parse(com.mojang.serialization.JsonOps.INSTANCE, saved).getOrThrow());
        var n = NfcSecret.componentType();
        var buf2 = new net.minecraft.network.RegistryFriendlyByteBuf(io.netty.buffer.Unpooled.buffer(), net.minecraft.core.RegistryAccess.EMPTY);
        n.streamCodec().encode(buf2, NfcSecret.factory());
        assertEquals(0, buf2.readableBytes());
    }
}
