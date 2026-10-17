package com.example.evanscomputermod.speaker;

import com.example.evanscomputermod.computer.wasi.DeviceFd;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.util.function.Supplier;

/**
 * {@code /dev/audio[.name]}: write PCM to a speaker. Each open is its own
 * {@link AudioStream} on the speaker's mixer ({@link SpeakerAudio}). Reading is not
 * supported.
 *
 * <p>The speaker is looked up again before every write ({@code target}): when the Volume
 * Mixer routes the program's app to another speaker, the stream moves there with its
 * settings, and the old one plays out what it had queued.
 */
public final class AudioDeviceFd implements DeviceFd {

    private final Supplier<SpeakerAudio> target;
    private final int pid;
    private final java.util.function.Consumer<AudioStream> onOpen;
    private AudioStream stream;
    private volatile boolean nonBlocking;

    /**
     * A stream for program {@code pid} on whatever speaker {@code target} names (it may
     * change). The stream starts with the first write: opening the device only to see that
     * it exists costs nothing.
     */
    public AudioDeviceFd(Supplier<SpeakerAudio> target, int pid, java.util.function.Consumer<AudioStream> onOpen) {
        this.target = target;
        this.pid = pid;
        this.onOpen = onOpen;
    }

    /** A stream for program {@code pid} on one fixed speaker. */
    public AudioDeviceFd(SpeakerAudio card, int pid) {
        this(() -> card, pid, s -> {});
    }

    /** The stream writes go to now, or null before the first write (tests, the Volume Mixer). */
    public synchronized AudioStream stream() {
        return stream;
    }

    private synchronized AudioStream current() throws SpeakerAudio.ClosedException {
        SpeakerAudio want = target.get();
        if (stream == null) {
            if (want == null) throw new SpeakerAudio.ClosedException();
            stream = want.openStream(pid);
            onOpen.accept(stream);
            return stream;
        }
        if (want != null && want != stream.card && !want.isClosed()) {
            AudioStream moved = want.openStream(pid);
            stream.copyTo(moved);
            stream.close();
            stream = moved;
        }
        return stream;
    }

    @Override
    public int read(byte[] buf, int offset, int length) throws IOException {
        throw new ErrnoException(ErrnoException.EINVAL, "/dev/audio is write-only");
    }

    @Override
    public int write(byte[] data, int offset, int length) throws IOException {
        try {
            return current().write(data, offset, length, nonBlocking);
        } catch (InterruptedException e) {
            // Killed while waiting for room: unwind like the other blocking calls.
            Thread.currentThread().interrupt();
            throw new RuntimeException("audio write interrupted");
        } catch (SpeakerAudio.ClosedException e) {
            throw new ErrnoException(ErrnoException.EIO, "speaker removed");
        }
    }

    @Override
    public synchronized void close() {
        if (stream != null) stream.close();
    }

    @Override public boolean isReadable() { return false; }
    @Override public boolean isWritable() { return true; }
    @Override public boolean isNonBlocking() { return nonBlocking; }
    @Override public void setNonBlocking(boolean nb) { this.nonBlocking = nb; }

    /**
     * {@code /dev/audioctl[.name]}: write commands ({@code rate 32768}, {@code bits 16},
     * {@code channels 1}, {@code volume 80}, {@code latency 100}, {@code role voice},
     * {@code flush}, one per line) for the opening program's streams; read their settings
     * and counters as {@code key value} lines. A read starts a fresh snapshot after EOF.
     */
    public static final class Ctl implements DeviceFd {
        private final Supplier<SpeakerAudio> target;
        private final int pid;
        private byte[] snapshot;
        private int pos;
        private final StringBuilder partial = new StringBuilder();

        /** The audioctl of program {@code pid} (0 = the card's own default stream). */
        public Ctl(Supplier<SpeakerAudio> target, int pid) {
            this.target = target;
            this.pid = pid;
        }

        /** The card's own default stream (tests, single-writer hosts). */
        public Ctl(SpeakerAudio audio) {
            this(() -> audio, 0);
        }

        @Override
        public synchronized int read(byte[] buf, int offset, int length) {
            if (snapshot == null) {
                snapshot = target.get().statusFor(pid).getBytes(StandardCharsets.UTF_8);
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
                target.get().controlFor(pid, line);
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
