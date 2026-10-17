package com.example.evanscomputermod.bluetooth;

import com.example.evanscomputermod.radio.api.Channel;
import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.List;
import java.util.UUID;

import static org.junit.jupiter.api.Assertions.*;

/** Bluetooth frames, pairing, connecting, audio both ways and link loss between a phone and AirPods. */
class BluetoothLinkTest {

    /** A shared air: frames reach the other side if it listens on that channel and {@link #reach} is on. */
    static final class Air {
        boolean reach = true;
        PhoneLink phone;
        HeadsetLink pods;
        Channel podsTuned, phoneData;
        final List<BtFrame> log = new ArrayList<>();
        long now;

        void fromPhone(BtFrame f, Channel data) {
            log.add(f);
            Channel ch = data == null ? BtAddress.ADV : data;
            if (reach && ch.equals(podsTuned)) pods.receive(BtFrame.decode(f.encode()), now);
        }

        void fromPods(BtFrame f, Channel data) {
            log.add(f);
            Channel ch = data == null ? BtAddress.ADV : data;
            // The phone always listens on the advertising channel, and on its link's data channel.
            if (reach && (ch.equals(BtAddress.ADV) || ch.equals(phoneData))) phone.receive(BtFrame.decode(f.encode()), -60, now);
        }

        void step(int ms, boolean micWanted) {
            for (int t = 0; t < ms; t += 50) {
                now += 50;
                pods.tick(now);
                phone.tick(now, micWanted);
            }
        }
    }

    final List<String> pairedLog = new ArrayList<>();
    final List<short[]> played = new ArrayList<>();
    final List<short[]> mic = new ArrayList<>();

    Air air() {
        Air air = new Air();
        long phoneAddr = BtAddress.of(UUID.randomUUID()), podsAddr = BtAddress.of(UUID.randomUUID());
        air.phone = new PhoneLink(phoneAddr, "Alex's ePhone", new PhoneLink.Radio() {
            @Override public void send(BtFrame f, Channel data) { air.fromPhone(f, data); }
            @Override public void tuneData(Channel data) { air.phoneData = data; }
        }, () -> new byte[] {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16});
        air.pods = new HeadsetLink(podsAddr, "AirPods", new HeadsetLink.Radio() {
            @Override public void send(BtFrame f, Channel data) { air.fromPods(f, data); }
            @Override public void tune(Channel c) { air.podsTuned = c; }
        }, new HeadsetLink.Owner() {
            @Override public void paired(long phone, byte[] key, String phoneName) { pairedLog.add(phoneName); }
            @Override public void play(short[] pcm, int rate) { played.add(pcm); }
        });
        air.phone.setMicSink((pcm, rate) -> mic.add(pcm));
        return air;
    }

    static short[] tone(int n) {
        short[] s = new short[n];
        for (int i = 0; i < n; i++) s[i] = (short) (Math.sin(i * 0.2) * 9000);
        return s;
    }

    @Test
    void framesRoundTrip() {
        BtFrame a = BtFrame.adv(0xACDE48001122L, new BtFrame.Adv(3, 1, 80, 255, 0x112233445566L, "AirPods"));
        BtFrame d = BtFrame.decode(a.encode());
        assertEquals(BtFrame.Type.ADV, d.type());
        assertEquals(0xACDE48001122L, d.from());
        assertEquals(new BtFrame.Adv(3, 1, 80, 255, 0x112233445566L, "AirPods"), d.adv());
        assertTrue(d.adv().pairable() && d.adv().connectable());
        byte[] key = new byte[16];
        key[3] = 7;
        BtFrame c = BtFrame.decode(BtFrame.connectRequest(1, 2, 9, key).encode());
        assertEquals(9, c.connectChannel());
        assertTrue(c.proves(key));
        assertFalse(c.proves(new byte[16]), "a wrong key doesn't prove anything");
        assertNull(BtFrame.decode(new byte[] {1, 2, 3}));
        assertEquals("AC:DE:48:00:11:22", BtAddress.format(0xACDE48001122L));
        assertEquals(0xACDE48001122L, BtAddress.parse("ac:de:48:00:11:22"));
        assertEquals(-1, BtAddress.parse("nope"));
        for (int n = 0; n < BtAddress.DATA_CHANNELS; n++) {
            Channel ch = BtAddress.data(n);
            assertFalse(ch.overlaps(BtAddress.ADV), "data channel " + n + " clear of advertising");
            assertNotEquals(2.426e9, ch.centerHz());
        }
    }

    @Test
    void pairConnectAndCarryAudioBothWays() {
        Air air = air();
        // Closed case: nothing to see; pairing is refused.
        air.step(400, false);
        assertTrue(air.phone.seen(air.now).isEmpty());
        assertNotNull(air.phone.pair(air.pods.addr, air.now));

        // Lid open: pairable.
        air.pods.setState(true, false, 90, 70);
        air.step(400, false);
        var seen = air.phone.seen(air.now);
        assertEquals(1, seen.size());
        assertEquals("AirPods", seen.get(0).name());
        assertTrue(seen.get(0).adv().pairable());
        assertNull(air.phone.pair(air.pods.addr, air.now));
        air.step(100, false);
        assertEquals(List.of("Alex's ePhone"), pairedLog);
        assertEquals(1, air.phone.paired().size());
        assertEquals(air.phone.addr, air.pods.pairedTo());

        // Worn: connected on a data channel; audio goes to the ears.
        air.pods.setState(false, true, 90, 255);
        air.step(600, false);
        assertEquals(air.pods.addr, air.phone.connected());
        assertTrue(air.pods.connected());
        assertTrue(air.phone.audioRouted());
        assertNotNull(air.phoneData);
        assertEquals(air.phoneData, air.podsTuned);
        air.phone.sendAudio(tone(2400), 24000);
        assertEquals(2, played.size(), "two blocks of 1600 + 800");
        assertEquals(1600, played.get(0).length);

        // The phone wants the microphone: the AirPods send it.
        air.step(500, true);
        assertTrue(air.pods.micWanted());
        air.pods.sendMic(tone(800), 16000);
        assertEquals(1, mic.size());
        assertEquals(800, mic.get(0).length);

        // Out of range: the link is lost, audio goes back to the speaker.
        air.reach = false;
        air.step(2600, false);
        assertEquals(-1, air.phone.connected());
        assertFalse(air.pods.connected());
        assertFalse(air.phone.audioRouted());
        // Back in range: it reconnects by itself (paired and worn).
        air.reach = true;
        air.step(800, false);
        assertEquals(air.pods.addr, air.phone.connected());

        // Disconnect by hand: stays off until connected again.
        air.phone.disconnect();
        air.step(800, false);
        assertEquals(-1, air.phone.connected());
        assertNull(air.phone.connect(air.pods.addr, air.now));
        air.step(500, false);
        assertEquals(air.pods.addr, air.phone.connected());

        // Taken off: the link drops; forgetting unpairs.
        air.pods.setState(false, false, 90, 255);
        air.step(100, false);
        assertFalse(air.pods.connected());
        air.phone.forget(air.pods.addr);
        assertTrue(air.phone.paired().isEmpty());
    }

    @Test
    void anotherPhoneCantConnectWithoutTheKey() {
        Air air = air();
        air.pods.setState(true, false, 90, 70);
        air.step(300, false);
        air.phone.pair(air.pods.addr, air.now);
        air.step(100, false);
        air.pods.setState(false, true, 90, 255);
        // A stranger phone asks with the wrong key: refused.
        List<BtFrame> answers = new ArrayList<>();
        air.pods.receive(BtFrame.decode(BtFrame.connectRequest(0x123456L, air.pods.addr, 3, new byte[16]).encode()), air.now);
        for (BtFrame f : air.log) if (f.type() == BtFrame.Type.CONNECT_RSP && f.to() == 0x123456L) answers.add(f);
        assertEquals(1, answers.size());
        assertFalse(answers.get(0).ok());
        assertNotEquals(0x123456L, air.pods.phone());
        // Pairing needs the lid open.
        air.pods.receive(BtFrame.decode(BtFrame.pairRequest(0x123456L, air.pods.addr, new byte[16], "thief").encode()), air.now);
        assertEquals(air.phone.addr, air.pods.pairedTo());
    }
}
