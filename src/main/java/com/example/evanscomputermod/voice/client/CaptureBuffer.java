package com.example.evanscomputermod.voice.client;

import java.util.concurrent.ConcurrentLinkedQueue;

/**
 * Microphone samples on their way from whichever capture is running (our own
 * OpenAL capture, Simple Voice Chat, Plasmo Voice) to the client tick that
 * sends them. Any thread may offer; no Minecraft classes, so a voice mod's
 * plugin can call it on either side.
 */
public final class CaptureBuffer {

    /** Where the samples came from. */
    public enum Source { OWN, SIMPLE_VOICE_CHAT, PLASMO_VOICE }

    /** One captured block: mono 16-bit samples at {@code rate}. */
    public record Block(short[] pcm, int rate, Source source) {}

    private static final ConcurrentLinkedQueue<Block> QUEUE = new ConcurrentLinkedQueue<>();
    /** Capture accepted while true (set by the client tick: microphone input on and something listening). */
    private static volatile boolean accepting;
    private static volatile int queued;

    private CaptureBuffer() {}

    static void setAccepting(boolean on) {
        accepting = on;
        if (!on) clear();
    }

    public static boolean accepting() {
        return accepting;
    }

    /** A capture produced samples (mono). Dropped unless the player's microphone input is on and wanted. */
    public static void offer(short[] pcm, int rate, Source source) {
        if (!accepting || pcm == null || pcm.length == 0 || rate < 8000 || rate > 96000) return;
        if (queued > 64) return;   // the client tick stalled: don't pile up seconds of audio
        QUEUE.add(new Block(pcm.clone(), rate, source));
        queued++;
    }

    /** Mix interleaved channels down to mono. */
    public static short[] mono(short[] pcm, int channels) {
        if (channels <= 1) return pcm;
        short[] out = new short[pcm.length / channels];
        for (int i = 0; i < out.length; i++) {
            int sum = 0;
            for (int c = 0; c < channels; c++) sum += pcm[i * channels + c];
            out[i] = (short) (sum / channels);
        }
        return out;
    }

    static Block poll() {
        Block b = QUEUE.poll();
        if (b != null) queued--;
        return b;
    }

    static void clear() {
        QUEUE.clear();
        queued = 0;
    }
}
