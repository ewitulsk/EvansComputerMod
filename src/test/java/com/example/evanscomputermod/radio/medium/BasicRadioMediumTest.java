package com.example.evanscomputermod.radio.medium;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Band;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.Reception;

import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.List;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicLong;

public class BasicRadioMediumTest {

    static final class Ep implements RadioEndpoint {
        UUID id = UUID.randomUUID();
        Pose pose;
        Channel ch;
        final List<Reception> got = new ArrayList<>();
        Ep(double x, Channel ch) { pose = Pose.at("overworld", x, 64, 0); this.ch = ch; }
        @Override public UUID id() { return id; }
        @Override public Pose pose() { return pose; }
        @Override public AntennaPattern antenna() { return AntennaPattern.ISOTROPIC; }
        @Override public Channel tunedChannel() { return ch; }
        @Override public double maxTxPowerDbm() { return 20; }
        @Override public void onReceive(Reception r) { got.add(r); }
    }

    @Test
    void freeSpaceLossMatchesFriis() {
        // 100 m at 2.4 GHz = 80.05 dB.
        assertEquals(80.05, BasicRadioMedium.freeSpaceLossDb(100, 2.4e9), 0.05);
        assertEquals(-101.0, BasicRadioMedium.noiseDbm(20e6, 0), 0.1);
    }

    @Test
    void wifiChannelsAndOverlap() {
        assertEquals(2.437e9, Channel.wifi24(6).centerHz(), 1);
        assertEquals(6, Channel.wifi24(6).wifiNumber());
        assertEquals(Band.WIFI_2G4, Channel.wifi24(1).band());
        assertTrue(Channel.wifi24(1).overlaps(Channel.wifi24(3)));
        assertFalse(Channel.wifi24(1).overlaps(Channel.wifi24(6)), "1/6/11 is the right plan");
        assertTrue(BasicRadioMedium.overlapFraction(Channel.wifi24(1), Channel.wifi24(2)) > 0.7);
        assertEquals(0, BasicRadioMedium.overlapFraction(Channel.wifi24(1), Channel.wifi24(11)));
    }

    @Test
    void deliversInRangeNotFarAndNotOtherChannels() {
        AtomicLong clock = new AtomicLong(1_000_000);
        BasicRadioMedium m = new BasicRadioMedium(clock::get, 42);
        Ep tx = new Ep(0, Channel.wifi24(6)), near = new Ep(30, Channel.wifi24(6)),
                far = new Ep(100_000, Channel.wifi24(6)), other = new Ep(30, Channel.wifi24(1));
        for (Ep e : List.of(tx, near, far, other)) m.register(e);
        m.transmit(tx, Emission.frame(Channel.wifi24(6), 15, clock.get(), 192, "DSSS-1", 1e6, new byte[100]));
        assertEquals(1, near.got.size());
        assertTrue(near.got.get(0).rssiDbm() < 15 - 60, "path loss applied");
        assertTrue(far.got.isEmpty(), "control: 100 km is beyond sensitivity");
        assertTrue(other.got.isEmpty(), "control: non-overlapping channel hears nothing");
        assertTrue(tx.got.isEmpty(), "no self-reception");
    }

    @Test
    void collisionAtReceiverDestroysWeakerFrame() {
        AtomicLong clock = new AtomicLong(5_000_000);
        BasicRadioMedium m = new BasicRadioMedium(clock::get, 7);
        Ch c = new Ch();
        Ep a = new Ep(0, c.ch), b = new Ep(400, c.ch), rx = new Ep(100, c.ch);
        for (Ep e : List.of(a, b, rx)) m.register(e);
        // Hidden-node style: b's long frame is on the air when a's frame starts.
        m.transmit(b, Emission.frame(c.ch, 20, clock.get(), 5000, "OFDM-54", 54e6, new byte[1500]));
        m.transmit(a, Emission.frame(c.ch, 20, clock.get(), 300, "OFDM-54", 54e6, new byte[1500]));
        long fromA = rx.got.stream().filter(r -> r.from().equals(a.id)).count();
        assertEquals(0, fromA, "54 Mb/s frame needs ~23 dB SINR; equal-ish interferer prevents it");
    }

    @Test
    void deterministicRolls() {
        for (int run = 0; run < 2; run++) {
            AtomicLong clock = new AtomicLong(1);
            BasicRadioMedium m = new BasicRadioMedium(clock::get, 99);
            Ep tx = new Ep(0, Channel.wifi24(11)), rx = new Ep(0, Channel.wifi24(11));
            tx.id = new UUID(1, 1);
            rx.id = new UUID(2, 2);
            rx.pose = Pose.at("overworld", 120, 64, 0);   // marginal link: SINR ~13 dB for a 13 dB modulation
            m.register(tx);
            m.register(rx);
            int count = 0;
            for (int i = 0; i < 200; i++) {
                clock.addAndGet(1000);
                m.transmit(tx, Emission.frame(Channel.wifi24(11), 0, clock.get(), 500, "OFDM-24", 24e6, new byte[] {(byte) i}));
            }
            count = rx.got.size();
            assertTrue(count > 100 && count < 200, "marginal link loses some frames: " + count);
            if (run == 0) first = count;
            else assertEquals(first, count);
        }
    }

    private int first;

    @Test
    void poseRotationRoundTrip() {
        float s = (float) Math.sin(Math.PI / 4), c = (float) Math.cos(Math.PI / 4);
        Pose p = new Pose("o", 0, 0, 0, 0, s, 0, c);   // 90° about Y
        double[] w = p.toWorld(0, 0, 1);
        assertEquals(1, w[0], 1e-6);
        assertEquals(0, w[2], 1e-6);
        double[] l = p.toLocal(w[0], w[1], w[2]);
        assertEquals(1, l[2], 1e-6);
        assertTrue(p.movedBeyond(Pose.at("o", 0, 0, 0), 0.5, Math.toRadians(2)));
        assertFalse(Pose.at("o", 0, 0, 0).movedBeyond(Pose.at("o", 0.1, 0, 0), 0.5, Math.toRadians(2)));
    }

    @Test
    void dipolePatternNullOnAxis() {
        assertEquals(2.15, AntennaPattern.VERTICAL_DIPOLE.gainDbi(1, 0, 0), 0.01);
        assertTrue(AntennaPattern.VERTICAL_DIPOLE.gainDbi(0, 1, 0) < -20);
    }

    private static final class Ch {
        final Channel ch = Channel.wifi24(6);
    }
}
