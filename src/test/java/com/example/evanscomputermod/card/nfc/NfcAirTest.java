package com.example.evanscomputermod.card.nfc;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.card.Hex;
import com.example.evanscomputermod.card.applet.NfcApplet;
import com.example.evanscomputermod.card.applet.NfcSecret;
import org.junit.jupiter.api.Test;

import java.util.Random;

/** The air frames and their codec, and the anticollision slot. */
public class NfcAirTest {

    @Test
    void frames() {
        NfcAir.Frame f = NfcAir.frame(NfcAir.AUTH, new byte[] {1}, new byte[8]);
        byte[] bytes = f.encode();
        assertEquals(11, bytes.length);
        assertEquals(NfcAir.AUTH, bytes[0]);
        assertEquals(9, bytes[1]);
        NfcAir.Frame back = NfcAir.decode(bytes);
        assertNotNull(back);
        assertEquals(f.cmd(), back.cmd());
        assertArrayEquals(f.body(), back.body());
        assertNull(NfcAir.decode(new byte[] {1}));
        assertNull(NfcAir.decode(new byte[] {1, 5, 0}), "length mismatch");
        assertTrue(NfcAir.error(NfcAir.ERR_NO_SESSION).isError());
        assertEquals(NfcAir.ERR_NO_SESSION, NfcAir.error(NfcAir.ERR_NO_SESSION).errorReason());
        assertEquals(Math.round(11 * 8 / 106_000.0 * 1e6), NfcAir.durationMicros(11));
        assertEquals(7, NfcAir.slotOf(0x0102030405060707L));
        assertEquals(15, NfcAir.slotOf(-1));
        assertTrue(NfcAir.onBand(NfcAir.CHANNEL.centerHz()));
    }

    /** The reader side of the handshake against the applet's answers, as frames. */
    @Test
    void readerSideHandshakeFrames() throws Exception {
        var mem = new com.example.evanscomputermod.card.applet.NfcAppletTest.Mem();
        NfcApplet n = new NfcApplet(mem);
        Random rnd = new Random(5);
        byte[] r1 = new byte[8];
        rnd.nextBytes(r1);
        NfcApplet.Challenge ch = n.challenge(3, r1, rnd);
        NfcAir.Frame reply = NfcAir.frame(NfcAir.AUTH | NfcAir.REPLY, ch.r2(), ch.m1());
        assertEquals(24, reply.body().length);
        byte[] r2 = reply.slice(0, 8), m1 = reply.slice(8, 16);
        assertArrayEquals(NfcApplet.m1(NfcSecret.FACTORY_KEY, r1, r2), m1);
        assertTrue(n.checkM2(3, r1, r2, NfcApplet.m2(NfcSecret.FACTORY_KEY, r1, r2)));
        assertEquals("45434d2d4e46432d464143544f525930", Hex.of(NfcSecret.FACTORY_KEY));
    }
}
