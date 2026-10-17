package com.example.evanscomputermod.radio.medium;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.bluetooth.BtAddress;
import com.example.evanscomputermod.bluetooth.BtEndpoint;
import com.example.evanscomputermod.bluetooth.BtFrame;
import com.example.evanscomputermod.bluetooth.HeadsetLink;
import com.example.evanscomputermod.bluetooth.PhoneLink;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Pose;

import org.junit.jupiter.api.Test;

import java.util.UUID;
import java.util.concurrent.atomic.AtomicLong;

/** A phone and AirPods over the real medium: advertising, pairing, the link, and range. */
public class BluetoothMediumTest {

    final AtomicLong clock = new AtomicLong(1_000_000);
    final GridWorld world = new GridWorld(64, RfBlock.DIRT);
    final WorldRadioMedium.WorldAccess access = new WorldRadioMedium.WorldAccess() {
        @Override public RfWorld world(String d) { return world; }
        @Override public WorldRadioMedium.Conditions conditions(String d) { return WorldRadioMedium.Conditions.DEFAULT; }
        @Override public Object subLevelOf(String d, double x, double y, double z) { return null; }
    };
    final WorldRadioMedium m = new WorldRadioMedium(clock::get, 7);
    long tick = 100, now = 10_000;

    BtEndpoint phoneAdv = new BtEndpoint(UUID.randomUUID()), phoneData = new BtEndpoint(UUID.randomUUID()), pods = new BtEndpoint(UUID.randomUUID());
    PhoneLink phone;
    HeadsetLink headset;
    int played;

    void setUp(double podsX) {
        phoneAdv.tune(BtAddress.ADV);
        phone = new PhoneLink(0x111111L, "phone", new PhoneLink.Radio() {
            @Override public void send(BtFrame f, Channel data) {
                if (data == null) phoneAdv.transmit(f, BtAddress.ADV);
                else phoneData.transmit(f, data);
            }
            @Override public void tuneData(Channel data) { phoneData.tune(data); }
        }, () -> new byte[16]);
        headset = new HeadsetLink(0x222222L, "AirPods", new HeadsetLink.Radio() {
            @Override public void send(BtFrame f, Channel data) { pods.transmit(f, data == null ? BtAddress.ADV : data); }
            @Override public void tune(Channel c) { pods.tune(c); }
        }, new HeadsetLink.Owner() {
            @Override public void paired(long p, byte[] key, String name) {}
            @Override public void play(short[] pcm, int rate) { played += pcm.length; }
        });
        for (BtEndpoint e : new BtEndpoint[] {phoneAdv, phoneData, pods}) e.sync(m);
        phoneAdv.moveTo(Pose.at("minecraft:overworld", 0.5, 66.3, 0.5));
        phoneData.moveTo(Pose.at("minecraft:overworld", 0.5, 66.3, 0.5));
        pods.moveTo(Pose.at("minecraft:overworld", podsX, 66.5, 0.5));
    }

    void step(int ticks, boolean pairable, boolean worn) {
        for (int i = 0; i < ticks; i++) {
            now += 50;
            clock.addAndGet(50_000);
            m.tick(tick++, access);
            headset.setState(pairable, worn, 100, 255);
            for (BtEndpoint.Heard h; (h = pods.poll()) != null; ) headset.receive(h.frame(), now);
            headset.tick(now);
            for (BtEndpoint.Heard h; (h = phoneAdv.poll()) != null; ) phone.receive(h.frame(), h.rssiDbm(), now);
            for (BtEndpoint.Heard h; (h = phoneData.poll()) != null; ) phone.receive(h.frame(), h.rssiDbm(), now);
            phone.tick(now, false);
        }
    }

    @Test
    void airpodsInTheSameHandsPairConnectAndPlay() {
        setUp(0.6);
        step(10, true, false);
        assertEquals(1, phone.seen(now).size(), "the phone hears the AirPods advertise over the medium");
        assertNull(phone.pair(0x222222L, now));
        step(10, true, false);
        assertEquals(1, phone.paired().size());
        step(10, false, true);
        assertEquals(0x222222L, phone.connected());
        phone.sendAudio(new short[1600], 24000);
        step(2, false, true);
        assertEquals(1600, played, "audio frames reach the AirPods on the data channel");
    }

    @Test
    void outOfRangeNothingIsHeard() {
        setUp(3000.5);
        step(20, true, false);
        assertTrue(phone.seen(now).isEmpty(), "3 km away: nothing");
    }
}
