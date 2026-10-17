package com.example.evanscomputermod.voice;

import com.example.evanscomputermod.computer.wasi.DeviceFd;
import com.example.evanscomputermod.computer.wasi.PipeFd;

import java.io.IOException;
import java.nio.charset.StandardCharsets;

/**
 * {@code /dev/mic[.name]}: read PCM from a microphone (see {@link MicAudio}).
 * Writing is not supported.
 */
public final class MicDeviceFd implements DeviceFd {

    private final MicAudio mic;
    /** This open file's own capture stream. */
    private final MicAudio.Stream stream;
    private volatile boolean nonBlocking;

    public MicDeviceFd(MicAudio mic) {
        this.mic = mic;
        this.stream = mic.openStream();
    }

    @Override
    public int read(byte[] buf, int offset, int length) throws IOException {
        Runnable l = mic.readListener();
        if (l != null) l.run();
        try {
            int n = mic.read(stream, buf, offset, length, nonBlocking);
            return n == MicAudio.WOULD_BLOCK ? PipeFd.WOULD_BLOCK : n;
        } catch (IllegalArgumentException e) {
            throw new ErrnoException(ErrnoException.EINVAL, e.getMessage());
        } catch (InterruptedException e) {
            // Killed while waiting for audio: unwind like the other blocking calls.
            Thread.currentThread().interrupt();
            throw new RuntimeException("microphone read interrupted");
        } catch (MicAudio.ClosedException e) {
            throw new ErrnoException(ErrnoException.EIO, "microphone removed");
        }
    }

    @Override
    public int write(byte[] data, int offset, int length) throws IOException {
        throw new ErrnoException(ErrnoException.EINVAL, "/dev/mic is read-only");
    }

    @Override public void close() {}
    @Override public boolean isReadable() { return true; }
    @Override public boolean isWritable() { return false; }
    @Override public boolean isNonBlocking() { return nonBlocking; }
    @Override public void setNonBlocking(boolean nb) { this.nonBlocking = nb; }

    /**
     * {@code /dev/micctl[.name]}: write commands ({@code rate 16000},
     * {@code bits 16}, {@code channels 1}, {@code gain 60}, {@code flush}, one
     * per line); read the settings and counters as {@code key value} lines. A
     * read starts a fresh snapshot after EOF.
     */
    public static final class Ctl implements DeviceFd {
        private final MicAudio mic;
        private final Runnable onGainChanged;
        private byte[] snapshot;
        private int pos;
        private final StringBuilder partial = new StringBuilder();

        public Ctl(MicAudio mic, Runnable onGainChanged) {
            this.mic = mic;
            this.onGainChanged = onGainChanged;
        }

        @Override
        public synchronized int read(byte[] buf, int offset, int length) {
            if (snapshot == null) {
                snapshot = mic.status().getBytes(StandardCharsets.UTF_8);
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
            if (partial.length() > 0 && partial.length() < 256) {
                String line = partial.toString();
                partial.setLength(0);
                apply(line);
            }
            snapshot = null;
            return length;
        }

        private void apply(String line) throws IOException {
            int before = mic.gain();
            try {
                mic.control(line, null);
            } catch (IllegalArgumentException e) {
                throw new ErrnoException(ErrnoException.EINVAL, e.getMessage());
            }
            if (mic.gain() != before && onGainChanged != null) onGainChanged.run();
        }

        @Override public void close() {}
        @Override public boolean isReadable() { return true; }
        @Override public boolean isWritable() { return true; }
        @Override public boolean isNonBlocking() { return false; }
        @Override public void setNonBlocking(boolean nb) {}
    }
}
