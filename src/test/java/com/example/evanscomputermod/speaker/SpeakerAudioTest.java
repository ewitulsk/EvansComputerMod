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
        assertEquals(40, a.volume());

        byte[] buf = new byte[256];
        int n = ctl.read(buf, 0, buf.length);
        String text = new String(buf, 0, n, StandardCharsets.UTF_8);
        assertTrue(text.contains("rate 32768\n") && text.contains("underruns 0\n"), text);
        assertEquals(0, ctl.read(buf, 0, buf.length), "EOF after the snapshot");

        byte[] bad = "rate 99\n".getBytes(StandardCharsets.UTF_8);
        var e = assertThrows(com.example.evanscomputermod.computer.wasi.DeviceFd.ErrnoException.class,
                () -> ctl.write(bad, 0, bad.length));
        assertEquals(28, e.errno());
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
