package com.example.evanscomputermod.speaker;

import org.junit.jupiter.api.Test;

import java.nio.charset.StandardCharsets;

import static org.junit.jupiter.api.Assertions.*;

/** The speaker's sound card: format conversion, the playback clock, ctl, and the network codec. */
class SpeakerAudioTest {

    private static byte[] s16(short... samples) {
        byte[] b = new byte[samples.length * 2];
        for (int i = 0; i < samples.length; i++) {
            b[i * 2] = (byte) samples[i];
            b[i * 2 + 1] = (byte) (samples[i] >> 8);
        }
        return b;
    }

    @Test
    void sixteenBitMonoPassesThrough() throws Exception {
        SpeakerAudio a = new SpeakerAudio();
        assertEquals(6, a.write(s16((short) 100, (short) -200, (short) 32767), 0, 6, false));
        assertArrayEquals(new short[]{100, -200, 32767}, a.drain());
        assertEquals(0, a.drain().length, "drain empties the queue");
    }

    @Test
    void eightBitStereoIsCentredAndMixedDown() throws Exception {
        SpeakerAudio a = new SpeakerAudio();
        a.setFormat(8000, 8, 2);
        // L=255, R=1 -> (127<<8 + -127<<8)/2 = 0; L=R=128 -> 0; L=R=0 -> -32768
        byte[] pcm = {(byte) 255, 1, (byte) 128, (byte) 128, 0, 0};
        a.write(pcm, 0, pcm.length, false);
        assertArrayEquals(new short[]{0, 0, -32768}, a.drain());
    }

    @Test
    void partialFramesCarryOverToTheNextWrite() throws Exception {
        SpeakerAudio a = new SpeakerAudio();
        byte[] one = s16((short) 0x1234);
        assertEquals(1, a.write(one, 0, 1, false));
        assertEquals(0, a.drain().length);
        assertEquals(1, a.write(one, 1, 1, false));
        assertArrayEquals(new short[]{0x1234}, a.drain());
    }

    @Test
    void volumeScalesSamples() throws Exception {
        SpeakerAudio a = new SpeakerAudio();
        a.setVolume(50);
        a.write(s16((short) 1000), 0, 2, false);
        assertArrayEquals(new short[]{500}, a.drain());
    }

    @Test
    void nonBlockingWritesStopAtTheLatencyBudget() throws Exception {
        SpeakerAudio a = new SpeakerAudio();
        a.setFormat(8000, 16, 1);
        a.setLatencyMs(100); // 800 samples
        byte[] second = new byte[8000 * 2];
        int took = a.write(second, 0, second.length, true);
        assertTrue(took >= 800 * 2 && took < 900 * 2, "took " + took + " bytes");
        assertTrue(a.bufferedMs() <= 100);
    }

    @Test
    void blockingWritesArePacedByTheAudioClock() throws Exception {
        SpeakerAudio a = new SpeakerAudio();
        a.setFormat(8000, 16, 1);
        a.setLatencyMs(50);
        byte[] quarterSecond = new byte[2000 * 2];
        long t0 = System.nanoTime();
        assertEquals(quarterSecond.length, a.write(quarterSecond, 0, quarterSecond.length, false));
        long ms = (System.nanoTime() - t0) / 1_000_000;
        // 250 ms of audio with a 50 ms queue: the write must wait ~200 ms for it to play.
        assertTrue(ms >= 150 && ms < 1000, "blocked " + ms + " ms");
    }

    @Test
    void closingWakesABlockedWriter() throws Exception {
        SpeakerAudio a = new SpeakerAudio();
        a.setFormat(8000, 16, 1);
        a.setLatencyMs(20);
        Exception[] caught = new Exception[1];
        Thread t = new Thread(() -> {
            try {
                a.write(new byte[16000 * 2], 0, 32000, false);
            } catch (Exception e) {
                caught[0] = e;
            }
        });
        t.start();
        Thread.sleep(100);
        a.close();
        t.join(2000);
        assertInstanceOf(SpeakerAudio.ClosedException.class, caught[0]);
    }

    @Test
    void underrunsAreCounted() throws Exception {
        SpeakerAudio a = new SpeakerAudio();
        a.setFormat(8000, 16, 1);
        a.write(new byte[80 * 2], 0, 160, false); // 10 ms
        Thread.sleep(40);
        a.write(new byte[80 * 2], 0, 160, false);
        assertEquals(1, a.underruns());
    }

    @Test
    void ctlReadsAndWritesSettings() throws Exception {
        SpeakerAudio a = new SpeakerAudio();
        AudioDeviceFd.Ctl ctl = new AudioDeviceFd.Ctl(a);
        byte[] cmd = "rate 32768\nbits 8\nchannels 2\nvolume 40\n".getBytes(StandardCharsets.UTF_8);
        ctl.write(cmd, 0, cmd.length);
        assertEquals(32768, a.rate());
        assertEquals(8, a.bits());
        assertEquals(2, a.channels());
        assertEquals(100, a.volume(), "audioctl's volume is the stream's; the speaker's own stays");

        byte[] buf = new byte[256];
        int n = ctl.read(buf, 0, buf.length);
        String text = new String(buf, 0, n, StandardCharsets.UTF_8);
        assertTrue(text.contains("rate 32768\n") && text.contains("underruns 0\n") && text.contains("volume 40\n"), text);
        assertTrue(text.contains("role media\n") && text.contains("ducked 0\n"), text);
        assertEquals(0, ctl.read(buf, 0, buf.length), "EOF after the snapshot");

        byte[] bad = "rate 99\n".getBytes(StandardCharsets.UTF_8);
        var e = assertThrows(com.example.evanscomputermod.computer.wasi.DeviceFd.ErrnoException.class,
                () -> ctl.write(bad, 0, bad.length));
        assertEquals(28, e.errno());
    }

    // ------------------------------------------------------------ the mixer

    private static byte[] tone(int rate, int n, int amp) {
        short[] s = new short[n];
        for (int i = 0; i < n; i++) s[i] = (short) (amp * Math.sin(2 * Math.PI * 440 * i / rate));
        return s16(s);
    }

    private static byte[] constant(int n, int v) {
        short[] s = new short[n];
        java.util.Arrays.fill(s, (short) v);
        return s16(s);
    }

    private static short[] drainAll(SpeakerAudio a, int ms) throws InterruptedException {
        java.util.List<Short> out = new java.util.ArrayList<>();
        long end = System.currentTimeMillis() + ms;
        while (System.currentTimeMillis() < end) {
            for (short v : a.drain()) out.add(v);
            Thread.sleep(10);
        }
        for (short v : a.drain()) out.add(v);
        short[] r = new short[out.size()];
        for (int i = 0; i < r.length; i++) r[i] = out.get(i);
        return r;
    }

    @Test
    void twoStreamsWithTheirOwnFormatsAreMixed() throws Exception {
        SpeakerAudio a = new SpeakerAudio();
        AudioStream x = a.openStream(1), y = a.openStream(2);
        x.setFormat(16000, 16, 1);
        y.setFormat(48000, 16, 1);
        x.setLatencyMs(300);
        y.setLatencyMs(300);
        x.write(constant(1600, 1000), 0, 3200, false);   // 100 ms at 16 kHz
        y.write(constant(4800, 2000), 0, 9600, false);   // 100 ms at 48 kHz
        short[] out = drainAll(a, 300);
        assertEquals(48000, a.outputRate(), "the mix runs at the fastest stream's rate");
        assertTrue(out.length >= 4700 && out.length <= 4900, "about 100 ms: " + out.length);
        // The middle of the window has both: 1000 + 2000.
        assertEquals(3000, out[out.length / 2], 30);
        assertNotEquals(x.rate(), y.rate(), "each stream keeps its own format");
    }

    @Test
    void aStreamThatJoinsLaterIsNotCutAndStartsAfterWhatWasSent() throws Exception {
        SpeakerAudio a = new SpeakerAudio();
        AudioStream music = a.openStream(1);
        music.setFormat(8000, 16, 1);
        music.setLatencyMs(400);
        music.write(constant(1600, 500), 0, 3200, false); // 200 ms
        short[] first = a.drain();
        assertTrue(first.length > 0);
        AudioStream voice = a.openStream(2);
        voice.setFormat(8000, 16, 1);
        voice.write(constant(400, 4000), 0, 800, false);   // 50 ms, joins now
        short[] rest = drainAll(a, 400);
        long voiced = 0;
        for (short v : rest) if (v == 4000) voiced++;
        // The music's 200 ms already went out: the late stream follows it, whole.
        assertEquals(400, voiced, 2, "all of the late stream is played after what was sent");
    }

    /**
     * The display pipeline's live bit (SpeakerAudioPacket.FLAG_LIVE): a drain that a call
     * (role voice or phone) put sound into is live; music alone, or a call that is silent, is
     * not. Each drain's pts is the mixer's own schedule, on the display clock.
     */
    @Test
    void aCallMakesTheDrainLiveAndMusicAloneDoesNot() throws Exception {
        assertTrue(SpeakerAudio.isLiveRole("voice") && SpeakerAudio.isLiveRole("phone"));
        assertFalse(SpeakerAudio.isLiveRole("media"));
        SpeakerAudio a = new SpeakerAudio();
        AudioStream music = a.openStream(1);
        music.setFormat(8000, 16, 1);
        music.setLatencyMs(1000);
        music.write(constant(1600, 3000), 0, 3200, false); // 200 ms
        long before = com.example.evanscomputermod.computer.pipeline.DisplayClock.nowUs();
        // /dev/audioctl's pts_us: a sample written now plays after the 200 ms queued.
        long next = Long.parseLong(music.status().lines().filter(l -> l.startsWith("pts_us ")).findFirst().orElseThrow().substring(7));
        assertTrue(next - before > 150_000 && next - before < 260_000, "pts_us " + (next - before) + " us ahead");
        assertTrue(a.drain().length > 0);
        assertFalse(a.lastDrainLive(), "music alone is not live");
        long pts = a.lastDrainPtsUs();
        assertTrue(pts >= before - 5_000 && pts <= before + 300_000, "pts on the display clock: " + (pts - before) + " us from now");
        // A silent call: still not live.
        AudioStream call = a.openStream(2);
        call.setFormat(8000, 16, 1);
        call.setRole(AudioStream.Role.VOICE);
        call.setLatencyMs(1000);
        call.write(constant(800, 0), 0, 1600, false);
        Thread.sleep(60);
        a.drain();
        assertFalse(a.lastDrainLive(), "a silent call is not live");
        // The call carries sound: live.
        call.write(constant(1600, 6000), 0, 3200, false);
        boolean live = false;
        for (int i = 0; i < 10 && !live; i++) {
            Thread.sleep(30);
            a.drain();
            live = a.lastDrainLive();
        }
        assertTrue(live, "a talking call makes the drain live");
    }

    @Test
    void voiceThatCarriesSoundDucksMusicAndASilentCallDoesNot() throws Exception {
        SpeakerAudio a = new SpeakerAudio();
        a.setDuckPercent(25);
        AudioStream music = a.openStream(1), call = a.openStream(2);
        music.setFormat(8000, 16, 1);
        call.setFormat(8000, 16, 1);
        call.setRole(AudioStream.Role.VOICE);
        music.setLatencyMs(1000);
        call.setLatencyMs(1000);
        // A silent call: music stays at full volume.
        music.write(constant(4000, 8000), 0, 8000, false);
        call.write(constant(4000, 0), 0, 8000, false);
        short[] quiet = drainAll(a, 600);
        assertFalse(music.ducked());
        assertEquals(8000, quiet[quiet.length / 2], 1);
        // The call carries sound: music falls to 25 % (plus the voice).
        music.write(constant(8000, 8000), 0, 16000, false);
        call.write(constant(8000, 1000), 0, 16000, false);
        short[] ducked = drainAll(a, 1100);
        assertTrue(music.ducked(), "a loud voice stream ducks media");
        assertEquals(8000 / 4 + 1000, ducked[ducked.length - 200], 40);
        assertTrue(a.streams().get(1).status().contains("role voice"));
    }

    @Test
    void perProgramAudioctlReachesOnlyThatProgramsStreams() throws Exception {
        SpeakerAudio a = new SpeakerAudio();
        AudioDeviceFd.Ctl ctl1 = new AudioDeviceFd.Ctl(() -> a, 7);
        byte[] cmd = "rate 22050\nrole voice\n".getBytes(StandardCharsets.UTF_8);
        ctl1.write(cmd, 0, cmd.length);
        AudioDeviceFd dev7 = new AudioDeviceFd(() -> a, 7, s -> {});
        AudioDeviceFd dev8 = new AudioDeviceFd(() -> a, 8, s -> {});
        assertNull(dev7.stream(), "opening only to look costs no stream");
        dev7.write(constant(10, 1), 0, 20);
        dev8.write(constant(10, 1), 0, 20);
        assertEquals(22050, dev7.stream().rate(), "settings given before the open apply");
        assertEquals(AudioStream.Role.VOICE, dev7.stream().role());
        assertEquals(48000, dev8.stream().rate(), "another program is untouched");
        assertEquals(AudioStream.Role.MEDIA, dev8.stream().role());
        dev7.close();
        assertTrue(dev7.stream().isClosed());
    }

    @Test
    void theVolumeMixersAppStripScalesAndMutes() throws Exception {
        SpeakerAudio a = new SpeakerAudio();
        AudioStream s = a.openStream(3);
        s.setFormat(8000, 16, 1);
        s.setMixer(50, false);
        s.write(constant(80, 2000), 0, 160, false);
        short[] half = drainAll(a, 50);
        assertEquals(1000, half[half.length / 2], 1);
        assertEquals(1000, s.peak(), 1, "the meter shows what is played");
        s.setMixer(150, true);
        s.write(constant(80, 2000), 0, 160, false);
        short[] muted = drainAll(a, 50);
        for (short v : muted) assertEquals(0, v);
        a.setMuted(true);
        s.setMixer(100, false);
        s.write(constant(80, 2000), 0, 160, false);
        for (short v : drainAll(a, 50)) assertEquals(0, v, "the master mute silences everything");
    }

    @Test
    void routingMovesAStreamToAnotherSpeaker() throws Exception {
        SpeakerAudio left = new SpeakerAudio(), right = new SpeakerAudio();
        SpeakerAudio[] target = {left};
        AudioDeviceFd dev = new AudioDeviceFd(() -> target[0], 4, s -> {});
        dev.write(constant(100, 300), 0, 200);
        assertSame(left, dev.stream().card);
        target[0] = right;
        dev.write(constant(100, 300), 0, 200);
        assertSame(right, dev.stream().card, "the next write goes to the new speaker");
        assertTrue(drainAll(left, 30).length > 0, "what was queued on the old one still plays");
        assertTrue(drainAll(right, 30).length > 0);
    }

    @Test
    void adpcmRoundTripIsClose() {
        short[] pcm = new short[4000];
        for (int i = 0; i < pcm.length; i++) {
            pcm[i] = (short) (12000 * Math.sin(2 * Math.PI * 440 * i / 32768.0));
        }
        ImaAdpcm enc = new ImaAdpcm();
        byte[] block = enc.encode(pcm, 0, pcm.length);
        assertEquals(ImaAdpcm.encodedSize(pcm.length), block.length);
        assertTrue(block.length <= pcm.length / 2 + ImaAdpcm.HEADER_BYTES, "4:1 against 16-bit");
        short[] out = ImaAdpcm.decode(block, pcm.length);
        double err = 0, sig = 0;
        for (int i = 0; i < pcm.length; i++) {
            err += Math.pow(pcm[i] - out[i], 2);
            sig += Math.pow(pcm[i], 2);
        }
        double snrDb = 10 * Math.log10(sig / err);
        assertTrue(snrDb > 20, "SNR " + snrDb + " dB");
    }

    @Test
    void adpcmRejectsMalformedBlocks() {
        assertNull(ImaAdpcm.decode(new byte[3], 4));
        assertNull(ImaAdpcm.decode(new byte[]{0, 0, (byte) 200, 0, 0}, 2), "step index out of range");
        assertNull(ImaAdpcm.decode(new byte[10], 0));
    }
}
