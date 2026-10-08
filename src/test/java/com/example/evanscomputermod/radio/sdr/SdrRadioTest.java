package com.example.evanscomputermod.radio.sdr;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.Reception;
import com.example.evanscomputermod.radio.medium.BasicRadioMedium;

import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.List;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicLong;

public class SdrRadioTest {

    static final class Ep implements RadioEndpoint {
        final UUID id = UUID.randomUUID();
        final Pose pose;
        SdrRadio radio;
        Ep(double x) { pose = Pose.at("o", x, 64, 0); }
        @Override public UUID id() { return id; }
        @Override public Pose pose() { return pose; }
        @Override public AntennaPattern antenna() { return AntennaPattern.VERTICAL_DIPOLE; }
        @Override public Channel tunedChannel() { return radio == null ? null : radio.channel(); }
        @Override public double maxTxPowerDbm() { return 37; }
        @Override public void onReceive(Reception r) {}
    }

    final AtomicLong clock = new AtomicLong(10_000_000);
    final BasicRadioMedium medium = new BasicRadioMedium(clock::get, 1);
    final List<String> events = new ArrayList<>();

    SdrRadio sdr(Ep ep, SdrTier tier, double hz) {
        SdrRadio r = new SdrRadio(tier, ep, clock::get, () -> 250_000, events::add, 5);
        ep.radio = r;
        r.setFrequency(hz);
        r.setSampleRate(48_000);
        medium.register(ep);
        return r;
    }

    /** Magnitude² of the DFT of {@code iq} at {@code hz}, normalised to mean power. */
    static double toneAmplitude(float[] iq, int n, double hz, double rate) {
        double re = 0, im = 0;
        for (int k = 0; k < n; k++) {
            double ph = -2 * Math.PI * hz * k / rate;
            re += iq[2 * k] * Math.cos(ph) - iq[2 * k + 1] * Math.sin(ph);
            im += iq[2 * k] * Math.sin(ph) + iq[2 * k + 1] * Math.cos(ph);
        }
        return Math.hypot(re, im) / n;
    }

    static float[] tone(int n, double hz, double rate) {
        float[] out = new float[2 * n];
        for (int k = 0; k < n; k++) {
            out[2 * k] = (float) Math.cos(2 * Math.PI * hz * k / rate);
            out[2 * k + 1] = (float) Math.sin(2 * Math.PI * hz * k / rate);
        }
        return out;
    }

    @Test
    void toneFromOneSdrIsHeardByAnotherAtTheRightOffset() {
        Ep a = new Ep(0), b = new Ep(10), c = new Ep(10);
        SdrRadio tx = sdr(a, SdrTier.STANDARD, 146.52e6);
        SdrRadio rx = sdr(b, SdrTier.STANDARD, 146.52e6);
        SdrRadio off = sdr(c, SdrTier.STANDARD, 147.52e6);   // control: 1 MHz away
        rx.setGain(0);
        off.setGain(0);
        float[] buf = new float[2 * 48_000];
        rx.read(medium, buf, 1);
        off.read(medium, buf, 1);
        tx.setTx(true, 0);   // 1 mW
        tx.write(medium, tone(4800, 5000, 48_000), 4800);
        clock.addAndGet(100_000);
        int n = rx.read(medium, buf, 48_000);
        assertTrue(n >= 4000, "read " + n);
        double at5k = toneAmplitude(buf, n, 5000, 48_000);
        double at9k = toneAmplitude(buf, n, -9000, 48_000);
        assertTrue(at5k > 20 * at9k, "tone at +5 kHz " + at5k + " vs elsewhere " + at9k);
        // 0 dBm - 35.8 dB (10 m at 146.52 MHz) + 4.3 dB antennas = -31.5 dBm against a -10 dBm full scale.
        double expectedAmp = Math.sqrt(Math.pow(10, (-31.5 + 10) / 10));
        assertEquals(expectedAmp, at5k, expectedAmp * 0.1);
        int m = off.read(medium, buf, 48_000);
        assertTrue(toneAmplitude(buf, m, 5000, 48_000) < at5k / 100, "control: off-frequency receiver heard the tone");
    }

    @Test
    void packetFrameShowsAsAnEnergyBurst() {
        Ep a = new Ep(0), b = new Ep(5);
        RadioEndpoint wifi = new Ep(3);
        medium.register(wifi);
        SdrRadio rx = sdr(b, SdrTier.ADVANCED, Channel.wifi24(6).centerHz());
        rx.setSampleRate(250_000);
        rx.setGain(0);
        float[] buf = new float[2 * 250_000];
        rx.read(medium, buf, 1);
        clock.addAndGet(10_000);
        int quiet = rx.read(medium, buf, 250_000);
        double quietPower = power(buf, quiet);
        medium.transmit(wifi, Emission.frame(Channel.wifi24(6), 20, clock.get(), 8000, "OFDM-6", 6e6, new byte[6000]));
        clock.addAndGet(8_000);
        int busy = rx.read(medium, buf, 250_000);
        assertTrue(power(buf, busy) > 1000 * quietPower, "burst " + power(buf, busy) + " vs quiet " + quietPower);
    }

    @Test
    void lateReaderOverflowsAndBasicCannotTransmit() {
        Ep a = new Ep(0);
        SdrRadio rx = sdr(a, SdrTier.BASIC, 100e6);
        float[] buf = new float[2 * 48_000];
        rx.read(medium, buf, 1);
        clock.addAndGet(2_000_000);   // 2 s, beyond the 0.25 s buffer
        rx.read(medium, buf, 48_000);
        assertTrue(events.contains("sdr_overflow"));
        assertThrows(IllegalArgumentException.class, () -> rx.setTx(true, 0));
        assertThrows(IllegalArgumentException.class, () -> rx.setFrequency(5e9), "basic tops out at 1.7 GHz");
        assertThrows(IllegalArgumentException.class, () -> rx.setSampleRate(250_000), "basic caps at 48 kS/s");
    }

    static double power(float[] iq, int n) {
        double p = 0;
        for (int k = 0; k < 2 * n; k++) p += iq[k] * iq[k];
        return p / Math.max(1, n);
    }
}
