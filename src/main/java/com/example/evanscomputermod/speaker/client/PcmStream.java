package com.example.evanscomputermod.speaker.client;

import net.minecraft.client.sounds.AudioStream;
import org.lwjgl.BufferUtils;

import javax.sound.sampled.AudioFormat;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * The audio received for one speaker, fed to Minecraft's sound engine as a
 * streaming sound (16-bit mono). The engine reads it on its own thread.
 *
 * <p>When nothing has arrived it plays short stretches of silence rather than
 * ending, so a pause in the music doesn't stop the sound. If audio piles up
 * (the client fell behind), the oldest is dropped to keep the delay short.
 */
final class PcmStream implements AudioStream {

    /** Keep at most this much queued (ms); older audio is dropped. */
    private static final int MAX_QUEUE_MS = 400;
    /** Silence handed out when nothing has arrived (ms). */
    private static final int SILENCE_MS = 50;

    private final int rate;
    private final AudioFormat format;
    private final short[] ring;
    private int head;
    private int size;
    private boolean closed;

    PcmStream(int rate) {
        this.rate = rate;
        this.format = new AudioFormat(rate, 16, 1, true, false);
        this.ring = new short[Math.max(1, rate * 2)];
    }

    int rate() {
        return rate;
    }

    synchronized void push(short[] pcm) {
        int max = rate * MAX_QUEUE_MS / 1000;
        for (short s : pcm) {
            if (size == ring.length) { // full: drop the oldest
                head = (head + 1) % ring.length;
                size--;
            }
            ring[(head + size) % ring.length] = s;
            size++;
        }
        if (size > max) {
            int drop = size - max;
            head = (head + drop) % ring.length;
            size -= drop;
        }
    }

    @Override
    public AudioFormat getFormat() {
        return format;
    }

    @Override
    public synchronized ByteBuffer read(int capacity) {
        if (closed) return null;
        int maxSamples = Math.max(1, capacity / 2);
        int n = Math.min(size, maxSamples);
        if (n == 0) {
            int silence = Math.min(maxSamples, Math.max(1, rate * SILENCE_MS / 1000));
            ByteBuffer buf = BufferUtils.createByteBuffer(silence * 2);
            buf.position(silence * 2).flip();
            return buf;
        }
        ByteBuffer buf = BufferUtils.createByteBuffer(n * 2).order(ByteOrder.LITTLE_ENDIAN);
        for (int i = 0; i < n; i++) {
            buf.putShort(ring[(head + i) % ring.length]);
        }
        head = (head + n) % ring.length;
        size -= n;
        buf.flip();
        return buf;
    }

    /** AudioStream (Closeable): the sound engine is done with it. */
    @Override
    public void close() {
        synchronized (this) {
            closed = true;
        }
    }
}
