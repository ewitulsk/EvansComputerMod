package com.example.evanscomputermod.speaker;

import com.example.evanscomputermod.computer.wasi.DeviceFd;

import java.io.IOException;
import java.nio.charset.StandardCharsets;

/**
 * {@code /dev/audio[.name]}: write PCM to a speaker (see {@link SpeakerAudio}).
 * Reading is not supported.
 */
public final class AudioDeviceFd implements DeviceFd {

    private final SpeakerAudio audio;
    private volatile boolean nonBlocking;

    public AudioDeviceFd(SpeakerAudio audio) {
        this.audio = audio;
    }

    @Override
    public int read(byte[] buf, int offset, int length) throws IOException {
        throw new ErrnoException(ErrnoException.EINVAL, "/dev/audio is write-only");
    }

    @Override
    public int write(byte[] data, int offset, int length) throws IOException {
        try {
            return audio.write(data, offset, length, nonBlocking);
        } catch (InterruptedException e) {
            // Killed while waiting for room: unwind like the other blocking calls.
            Thread.currentThread().interrupt();
            throw new RuntimeException("audio write interrupted");
        } catch (SpeakerAudio.ClosedException e) {
            throw new ErrnoException(ErrnoException.EIO, "speaker removed");
        }
    }

    @Override public void close() {}
    @Override public boolean isReadable() { return false; }
    @Override public boolean isWritable() { return true; }
    @Override public boolean isNonBlocking() { return nonBlocking; }
    @Override public void setNonBlocking(boolean nb) { this.nonBlocking = nb; }

    /**
     * {@code /dev/audioctl[.name]}: write commands ({@code rate 32768},
     * {@code bits 16}, {@code channels 1}, {@code volume 80}, {@code latency 100},
     * {@code flush}, one per line); read the settings and counters as
     * {@code key value} lines. A read starts a fresh snapshot after EOF.
     */
    public static final class Ctl implements DeviceFd {
        private final SpeakerAudio audio;
        private byte[] snapshot;
        private int pos;
        private final StringBuilder partial = new StringBuilder();

        public Ctl(SpeakerAudio audio) {
            this.audio = audio;
        }

        @Override
        public synchronized int read(byte[] buf, int offset, int length) {
            if (snapshot == null) {
                snapshot = audio.status().getBytes(StandardCharsets.UTF_8);
                pos = 0;
            }
            int n = Math.min(length, snapshot.length - pos);
            System.arraycopy(snapshot, pos, buf, offset, n);
            pos += n;
            if (n == 0) snapshot = null; // EOF; the next read starts over
            return n;
        }

        @Override
        public synchronized int write(byte[] data, int offset, int length) throws IOException {
            partial.append(new String(data, offset, length, StandardCharsets.UTF_8));
            int nl;
            while ((nl = partial.indexOf("\n")) >= 0) {
                String line = partial.substring(0, nl);
                partial.delete(0, nl + 1);
                apply(line);
            }
            // A command written without a newline still counts.
            if (partial.length() > 0 && partial.length() < 256) {
                String line = partial.toString();
                partial.setLength(0);
                apply(line);
            }
            snapshot = null;
            return length;
        }

        private void apply(String line) throws IOException {
            try {
                audio.control(line);
            } catch (IllegalArgumentException e) {
                throw new ErrnoException(ErrnoException.EINVAL, e.getMessage());
            }
        }

        @Override public void close() {}
        @Override public boolean isReadable() { return true; }
        @Override public boolean isWritable() { return true; }
        @Override public boolean isNonBlocking() { return false; }
        @Override public void setNonBlocking(boolean nb) {}
    }
}
