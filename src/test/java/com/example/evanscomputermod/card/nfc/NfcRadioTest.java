package com.example.evanscomputermod.card.nfc;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.card.Hex;
import com.example.evanscomputermod.card.applet.NfcApplet;
import com.example.evanscomputermod.card.applet.NfcAppletTest;
import com.example.evanscomputermod.card.applet.NfcSecret;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.medium.GridWorld;
import com.example.evanscomputermod.radio.medium.RfBlock;
import com.example.evanscomputermod.radio.medium.RfWorld;
import com.example.evanscomputermod.radio.medium.WorldRadioMedium;
import org.junit.jupiter.api.Test;

import java.util.Map;
import java.util.Random;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicLong;

/**
 * An NFC reader and a passive card over the real {@link WorldRadioMedium}: the whole air
 * protocol end to end (poll, select, the 3-pass handshake, read, write, rekey), the range
 * (1 block yes, 4 blocks no), a cold field, and an iron block in the way.
 */
public class NfcRadioTest {
    static final String DIM = "minecraft:overworld";

    final AtomicLong clock = new AtomicLong(1_000_000);
    final GridWorld world = new GridWorld(64, RfBlock.DIRT);
    final WorldRadioMedium.WorldAccess access = new WorldRadioMedium.WorldAccess() {
        @Override public RfWorld world(String d) { return world; }
        @Override public WorldRadioMedium.Conditions conditions(String d) { return WorldRadioMedium.Conditions.DEFAULT; }
    };
    final WorldRadioMedium m = new WorldRadioMedium(clock::get, 11);
    long tick = 100;

    /** An in-memory NFC card. */
    public static final class MemCard implements NfcCard {
        final NfcAppletTest.Mem mem = new NfcAppletTest.Mem();
        final NfcApplet applet = new NfcApplet(mem);

        public MemCard(long uid) {
            mem.uid = uid;
        }

        @Override public NfcApplet nfc() { return applet; }
        @Override public long uid() { return mem.uid; }
        @Override public int kinds() { return NfcAir.KIND_NFC; }
    }

    /** Game ticks: the medium traces links and the airtime clock moves on 50 ms (stale frames leave the air). */
    void ticks(int n) {
        for (int i = 0; i < n; i++) {
            m.tick(tick++, access);
            clock.addAndGet(50_000);
        }
    }

    NfcRadio reader(double x) {
        NfcRadio r = new NfcRadio(UUID.randomUUID(), () -> Pose.at(DIM, x, 65.5, 0.5), () -> m, 10);
        r.load();
        return r;
    }

    CardEndpoint card(MemCard c, double x) {
        CardEndpoint ep = new CardEndpoint(c, Pose.at(DIM, x, 65.5, 0.5), clock::get, new Random(1));
        ep.attach(m);
        m.register(ep);
        return ep;
    }

    @Test
    void fullProtocolAtOneBlock() {
        NfcRadio r = reader(0.5);
        MemCard c = new MemCard(0x0102030405060708L);
        CardEndpoint ep = card(c, 1.5);
        r.setField(true);
        ticks(3);
        Map<Long, Integer> found = r.poll(2);
        assertEquals(Map.of(0x0102030405060708L, NfcAir.KIND_NFC), found);
        ticks(1);
        assertTrue(ep.energised());
        assertTrue(r.select(0x0102030405060708L));
        assertNull(r.auth(1, NfcSecret.FACTORY_KEY, new Random(2)), "the factory key opens sector 1");
        NfcAir.Frame read = r.one(NfcAir.frame(NfcAir.READ, new byte[] {1}));
        assertNotNull(read);
        assertEquals(NfcAir.READ | NfcAir.REPLY, read.cmd());
        assertEquals(64, read.body().length);
        byte[] data = new byte[64];
        java.util.Arrays.fill(data, (byte) 0x5A);
        NfcAir.Frame w = r.one(NfcAir.frame(NfcAir.WRITE, new byte[] {1}, data));
        assertNotNull(w);
        assertEquals(NfcAir.WRITE | NfcAir.REPLY, w.cmd());
        assertArrayEquals(data, Hex.parse(c.mem.secret.dataHex().get(1), 64, "d"));
        NfcAir.Frame noSession = r.one(NfcAir.frame(NfcAir.READ, new byte[] {2}));
        assertNotNull(noSession);
        assertTrue(noSession.isError());
        assertEquals(NfcAir.ERR_NO_SESSION, noSession.errorReason());
        byte[] key = Hex.parse("00112233445566778899aabbccddeeff", 16, "k");
        NfcAir.Frame sk = r.one(NfcAir.frame(NfcAir.SETKEY, new byte[] {1}, key));
        assertEquals(NfcAir.SETKEY | NfcAir.REPLY, sk.cmd());
        assertEquals("the card does not have this key", r.auth(1, NfcSecret.FACTORY_KEY, new Random(3)), "rekeyed");
        assertNull(r.auth(1, key, new Random(4)));
        assertTrue(r.lastRssi() >= NfcRadio.SENSITIVITY_DBM);
        assertTrue(ep.answered >= 8);
    }

    @Test
    void outOfRangeAndColdField() {
        NfcRadio r = reader(0.5);
        MemCard near = new MemCard(1), far = new MemCard(2);
        CardEndpoint epNear = card(near, 1.5), epFar = card(far, 4.5);
        ticks(3);
        assertTrue(r.poll(1).isEmpty(), "the field is off: nothing answers");
        r.setField(true);
        ticks(3);
        Map<Long, Integer> found = r.poll(2);
        assertTrue(found.containsKey(1L), "1 block: heard");
        assertFalse(found.containsKey(2L), "4 blocks: never energised");
        assertFalse(epFar.energised());
        assertEquals(0, epFar.answered);
        r.setField(false);
        clock.addAndGet(500_000);
        assertFalse(epNear.energised(), "the hold lapsed");
        assertTrue(r.poll(1).isEmpty());
    }

    @Test
    void ironBlockShields() {
        NfcRadio r = reader(0.5);
        MemCard open = new MemCard(3), shielded = new MemCard(4);
        // Replies carry only field − 20 dB, so a 10 dBm reader reads cards to about 1.3 blocks but
        // energises them to about 2.3: the shield shows in whether the card wakes at all.
        CardEndpoint epOpen = card(open, 2.2);
        CardEndpoint epS = new CardEndpoint(shielded, Pose.at(DIM, 0.5, 65.5, 2.2), clock::get, new Random(1));
        epS.attach(m);
        m.register(epS);
        world.set(0, 65, 1, RfBlock.IRON);
        r.setField(true);
        ticks(3);
        r.poll(1);
        boolean afterPoll = epOpen.energised();
        m.transmit(r.endpoint(), com.example.evanscomputermod.radio.api.Emission.energy(NfcAir.CHANNEL, 10, clock.get(), 50_000));
        assertTrue(epOpen.energised(), "1.7 blocks of air (after poll: " + afterPoll + ", endpoints " + m.endpointCount() + "): path gain " + m.pathGainDb(r.endpoint(), epOpen, NfcAir.CHANNEL.centerHz())
                + " dB, link " + m.link(r.endpoint(), epOpen, NfcAir.CHANNEL.centerHz()));
        assertFalse(epS.energised(), "1.7 blocks through iron");
    }

    @Test
    void twoCardsAnticollision() {
        NfcRadio r = reader(0.5);
        card(new MemCard(0x10), 1.1);
        card(new MemCard(0x21), 1.2);
        r.setField(true);
        ticks(3);
        Map<Long, Integer> found = r.poll(4);
        assertEquals(2, found.size(), "different slots: both heard " + found);
        ticks(1);
        assertTrue(r.select(0x21), "select after a tick");
        ticks(1);
        assertTrue(r.select(0x10));
        assertFalse(r.select(0x99), "no such card");
    }
}
