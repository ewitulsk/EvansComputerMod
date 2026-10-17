package com.example.evanscomputermod.voice;

import com.example.evanscomputermod.radio.handheld.HandheldDemod;
import org.junit.jupiter.api.Test;

import java.util.concurrent.atomic.AtomicLong;

import static org.junit.jupiter.api.Assertions.*;

/** The microphone's sound card, the sound field it hears, and the voice modulator. */
class MicAudioTest {

    /** A clock tests move by hand. */
    private static final class Clock {
        final AtomicLong nanos = new AtomicLong(1_000_000_000L);

        long get() { return nanos.get(); }

        void advanceMs(long ms) { nanos.addAndGet(ms * 1_000_000L); }
    }

    private static short[] tone(double hz, int rate, int n, double amp, int offset) {
        short[] s = new short[n];
        for (int i = 0; i < n; i++) s[i] = (short) Math.round(Math.sin(2 * Math.PI * hz * (i + offset) / rate) * amp * 32767);
        return s;
    }

    private static double rms16(byte[] b, int len) {
        double sum = 0;
        int n = len / 2;
        for (int i = 0; i < n; i++) {
            short v = (short) ((b[2 * i] & 0xFF) | (b[2 * i + 1] << 8));
            sum += (v / 32768.0) * (v / 32768.0);
        }
        return Math.sqrt(sum / Math.max(1, n));
    }

    @Test
    void captureClockPacesReadsAndQuietRoomIsOnlyHiss() throws Exception {
        Clock c = new Clock();
        MicAudio mic = new MicAudio((s, n, r, out) -> {}, c::get, 1);
        mic.setFormat(16000, 16, 1);
        byte[] buf = new byte[3200];
        assertEquals(MicAudio.WOULD_BLOCK, mic.read(buf, 0, buf.length, true), "nothing captured before capture starts");
        c.advanceMs(50);
        int n = mic.read(buf, 0, buf.length, true);
        assertEquals(800 * 2, n, "50 ms at 16 kHz is 800 frames");
        double r = rms16(buf, n);
        assertTrue(r > 1e-5 && r < 1e-3, "self-noise only, got rms " + r);
        assertEquals(MicAudio.WOULD_BLOCK, mic.read(buf, 0, buf.length, true), "everything captured was read");
    }

    @Test
    void readerThatFallsBehindOverruns() throws Exception {
        Clock c = new Clock();
        MicAudio mic = new MicAudio((s, n, r, out) -> {}, c::get, 1);
        byte[] buf = new byte[96000];
        c.advanceMs(1);
        mic.read(buf, 0, 2, true);
        c.advanceMs(3000);
        mic.read(buf, 0, buf.length, true);
        assertEquals(1, mic.overruns());
        assertTrue(mic.status().contains("overruns 1"));
    }

    @Test
    void twoProgramsRecordingAtOnceBothGetEverything() throws Exception {
        Clock c = new Clock();
        MicAudio mic = new MicAudio((s, n, r, out) -> {}, c::get, 1);
        mic.setFormat(8000, 16, 1);
        MicAudio.Stream a = mic.openStream(), b = mic.openStream();
        byte[] buf = new byte[1600];
        mic.read(a, buf, 0, 2, true);
        mic.read(b, buf, 0, 2, true);
        c.advanceMs(100);
        assertEquals(1600, mic.read(a, buf, 0, buf.length, true));
        assertEquals(1600, mic.read(b, buf, 0, buf.length, true), "the second reader isn't starved by the first");
        c.advanceMs(100);
        mic.control("flush", null);
        assertEquals(MicAudio.WOULD_BLOCK, mic.read(a, buf, 0, buf.length, true), "a ctl flush drops every stream's backlog");
        assertEquals(MicAudio.WOULD_BLOCK, mic.read(b, buf, 0, buf.length, true));
    }

    @Test
    void gainAndFormatThroughCtlCommands() {
        MicAudio mic = new MicAudio((s, n, r, out) -> {});
        mic.control("rate 8000");
        mic.control("bits 8");
        mic.control("channels 2");
        mic.control("gain 75");
        assertEquals(8000, mic.rate());
        assertEquals(8, mic.bits());
        assertEquals(2, mic.channels());
        assertEquals(75, mic.gain());
        assertEquals(10.0, MicAudio.gainFactor(100), 1e-9);
        assertEquals(1.0, MicAudio.gainFactor(50), 1e-9);
        assertEquals(0.0, MicAudio.gainFactor(0));
        assertThrows(IllegalArgumentException.class, () -> mic.control("rate 100"));
        assertThrows(IllegalArgumentException.class, () -> mic.control("bits 12"));
        assertThrows(IllegalArgumentException.class, () -> mic.control("loud 3"));
    }

    @Test
    void microphoneHearsANearbySourceFainterWithDistance() throws Exception {
        Clock c = new Clock();
        SoundField field = new SoundField(c::get);
        MicAudio near = new MicAudio((s, n, r, out) -> field.render("w", 1, 0, 0, s, n, r, out), c::get, 1);
        MicAudio far = new MicAudio((s, n, r, out) -> field.render("w", 8, 0, 0, s, n, r, out), c::get, 2);
        MicAudio other = new MicAudio((s, n, r, out) -> field.render("nether", 1, 0, 0, s, n, r, out), c::get, 3);
        byte[] b = new byte[2];
        near.read(b, 0, 2, true);
        far.read(b, 0, 2, true);
        other.read(b, 0, 2, true);
        // A speaker at the origin plays a 1 kHz tone at half scale, in 50 ms bursts.
        for (int t = 0; t < 20; t++) {
            field.emit("speaker", "w", 0, 0, 0, tone(1000, 48000, 2400, 0.5, t * 2400), 0, 2400, 48000, 1f);
            c.advanceMs(50);
        }
        byte[] buf = new byte[48000 * 2];
        int n1 = near.read(buf, 0, buf.length, true);
        double rNear = rms16(buf, n1);
        int n2 = far.read(buf, 0, buf.length, true);
        double rFar = rms16(buf, n2);
        int n3 = other.read(buf, 0, buf.length, true);
        double rOther = rms16(buf, n3);
        // Half-scale sine: RMS 0.354 heard after the 100 ms latency, at 1 block (full level).
        assertTrue(rNear > 0.25, "near mic hears the tone, rms " + rNear);
        assertEquals(1 / 8.0, rFar / rNear, 0.03, "8 blocks away is 1/8 the level");
        assertTrue(rOther < 1e-3, "another dimension hears only hiss, rms " + rOther);
    }

    @Test
    void sourceOutOfRangeIsNotHeard() throws Exception {
        Clock c = new Clock();
        SoundField field = new SoundField(c::get);
        field.setRange(16);
        MicAudio mic = new MicAudio((s, n, r, out) -> field.render("w", 20, 0, 0, s, n, r, out), c::get, 1);
        byte[] b = new byte[2];
        mic.read(b, 0, 2, true);
        for (int t = 0; t < 10; t++) {
            field.emit("voice", "w", 0, 0, 0, tone(440, 24000, 1200, 0.9, t * 1200), 0, 1200, 24000, 1f);
            c.advanceMs(50);
        }
        byte[] buf = new byte[96000];
        int n = mic.read(buf, 0, buf.length, true);
        assertTrue(rms16(buf, n) < 1e-3);
    }

    @Test
    void burstsAreJoinedBackToBackAndAQuietSourceRestarts() {
        Clock c = new Clock();
        SoundField field = new SoundField(c::get);
        long now = field.nowAir();
        field.emit("v", "w", 0, 0, 0, new short[2400], 0, 2400, 48000, 1f);
        AirTrack t = field.find("v");
        assertEquals(now + AirTrack.LATENCY + 2400, t.head(), "first burst lands one latency after it arrived");
        c.advanceMs(30);
        field.emit("v", "w", 0, 0, 0, new short[2400], 0, 2400, 48000, 1f);
        assertEquals(now + AirTrack.LATENCY + 4800, t.head(), "an early burst follows the previous one");
        c.advanceMs(2000);
        long later = field.nowAir();
        field.emit("v", "w", 0, 0, 0, new short[1200], 0, 1200, 24000, 1f);
        assertEquals(later + AirTrack.LATENCY + 2400, t.head(), "after silence it starts again; 24 kHz doubles to the air rate");
    }

    @Test
    void fmVoiceSurvivesTheHandheldReceiver() {
        // 1 kHz at 24 kHz audio, narrow FM at 48 kS/s, demodulated by the handheld's discriminator.
        VoiceModulator mod = new VoiceModulator(VoiceModulator.Mode.FM, 24000, 48000, 5000);
        HandheldDemod demod = new HandheldDemod();
        int n = 24000;
        float[] audio = new float[n];
        for (int i = 0; i < n; i++) audio[i] = (float) (0.5 * Math.sin(2 * Math.PI * 1000 * i / 24000.0));
        float[] iq = mod.modulate(audio);
        assertEquals(2 * 48000, iq.length);
        double rmsIq = 0;
        for (int i = 0; i < 48000; i++) rmsIq += iq[2 * i] * iq[2 * i] + iq[2 * i + 1] * iq[2 * i + 1];
        assertEquals(1.0, rmsIq / 48000, 1e-3, "FM is constant envelope at RMS 1");
        float[] out = demod.fm(iq, 48000, 2, 48000, 5000);
        // Correlate the second half (filters settled) with a 1 kHz sine and cosine.
        double s = 0, co = 0, e = 0;
        for (int i = n / 2; i < n; i++) {
            s += out[i] * Math.sin(2 * Math.PI * 1000 * i / 24000.0);
            co += out[i] * Math.cos(2 * Math.PI * 1000 * i / 24000.0);
            e += out[i] * out[i];
        }
        double toneEnergy = 2 * (s * s + co * co) / (n / 2.0);
        assertTrue(toneEnergy / e > 0.9, "the demodulated audio is the 1 kHz tone (" + toneEnergy / e + ")");
        assertTrue(Math.sqrt(e / (n / 2.0)) > 0.1, "at a usable level");
    }

    @Test
    void amVoiceSurvivesTheHandheldReceiver() {
        VoiceModulator mod = new VoiceModulator(VoiceModulator.Mode.AM, 24000, 48000, 0);
        HandheldDemod demod = new HandheldDemod();
        int n = 48000;
        float[] audio = new float[n];
        for (int i = 0; i < n; i++) audio[i] = (float) (0.6 * Math.sin(2 * Math.PI * 700 * i / 24000.0));
        float[] iq = mod.modulate(audio);
        float[] out = demod.am(iq, 2 * n, 2, 1.5);
        double s = 0, co = 0, e = 0;
        for (int i = n / 2; i < n; i++) {
            s += out[i] * Math.sin(2 * Math.PI * 700 * i / 24000.0);
            co += out[i] * Math.cos(2 * Math.PI * 700 * i / 24000.0);
            e += out[i] * out[i];
        }
        double toneEnergy = 2 * (s * s + co * co) / (n / 2.0);
        assertTrue(toneEnergy / e > 0.9, "the AM envelope is the 700 Hz tone (" + toneEnergy / e + ")");
    }
}
