package com.example.evanscomputermod.radio.sdr;

import com.example.evanscomputermod.computer.wasi.DeviceFd;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;

/**
 * {@code /dev/sdr[N|.name]}: read receive samples, write transmit samples, as
 * interleaved little-endian {@code cs16} (int16 I,Q) or {@code cf32} (float I,Q).
 * Reads block until samples exist on the world clock (non-blocking reads
 * return 0); a partial sample is never returned.
 */
public final class SdrDeviceFd implements DeviceFd {

    private final SdrPeripheral sdr;
    private volatile boolean nonBlocking;

    public SdrDeviceFd(SdrPeripheral sdr) {
        this.sdr = sdr;
    }

    private int bytesPerSample() {
        return sdr.radio().format() == SdrRadio.Format.CF32 ? 8 : 4;
    }

    @Override
    public int read(byte[] buf, int offset, int length) throws IOException {
        SdrRadio r = sdr.radio();
        int bps = bytesPerSample();
        int max = length / bps;
        if (max <= 0) throw new ErrnoException(ErrnoException.EINVAL, "buffer smaller than one sample");
        float[] iq = new float[2 * max];
        int n;
        long deadline = System.nanoTime() + 2_000_000_000L;
        while ((n = r.read(sdr.medium(), iq, max)) == 0) {
            if (nonBlocking) return 0;
            if (Thread.currentThread().isInterrupted() || System.nanoTime() > deadline) return 0;
            try {
                Thread.sleep(2);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                throw new RuntimeException("sdr read interrupted");
            }
        }
        ByteBuffer out = ByteBuffer.wrap(buf, offset, n * bps).order(ByteOrder.LITTLE_ENDIAN);
        for (int k = 0; k < 2 * n; k++) {
            if (bps == 8) out.putFloat(iq[k]);
            else out.putShort((short) Math.round(Math.max(-1, Math.min(1, iq[k])) * 32767));
        }
        return n * bps;
    }

    @Override
    public int write(byte[] data, int offset, int length) throws IOException {
        SdrRadio r = sdr.radio();
        int bps = bytesPerSample();
        int n = length / bps;
        if (n <= 0) return 0;
        ByteBuffer in = ByteBuffer.wrap(data, offset, n * bps).order(ByteOrder.LITTLE_ENDIAN);
        float[] iq = new float[2 * n];
        for (int k = 0; k < 2 * n; k++) iq[k] = bps == 8 ? in.getFloat() : in.getShort() / 32767f;
        try {
            r.write(sdr.medium(), iq, n);
        } catch (IllegalStateException e) {
            throw new ErrnoException(ErrnoException.EIO, e.getMessage());
        }
        return n * bps;
    }

    @Override public void close() {}
    @Override public boolean isReadable() { return true; }
    @Override public boolean isWritable() { return true; }
    @Override public boolean isNonBlocking() { return nonBlocking; }
    @Override public void setNonBlocking(boolean nb) { nonBlocking = nb; }

    /**
     * {@code /dev/sdrctl[N|.name]}: write commands one per line ({@code freq 146520000},
     * {@code rate 48000}, {@code gain 20|agc}, {@code bw 12500}, {@code format cs16|cf32},
     * {@code tx on [dBm]|off}); read the settings as {@code key value} lines.
     */
    public static final class Ctl implements DeviceFd {
        private final SdrPeripheral sdr;
        private byte[] snapshot;
        private int pos;
        private final StringBuilder partial = new StringBuilder();
        private volatile boolean nonBlocking;

        public Ctl(SdrPeripheral sdr) {
            this.sdr = sdr;
        }

        @Override
        public synchronized int read(byte[] buf, int offset, int length) {
            if (snapshot == null) {
                snapshot = sdr.radio().describe().getBytes(StandardCharsets.UTF_8);
                pos = 0;
            }
            int n = Math.min(length, snapshot.length - pos);
            if (n <= 0) {
                snapshot = null;
                return 0;
            }
            System.arraycopy(snapshot, pos, buf, offset, n);
            pos += n;
            return n;
        }

        @Override
        public synchronized int write(byte[] data, int offset, int length) throws IOException {
            partial.append(new String(data, offset, length, StandardCharsets.UTF_8));
            int nl;
            while ((nl = partial.indexOf("\n")) >= 0) {
                String line = partial.substring(0, nl);
                partial.delete(0, nl + 1);
                try {
                    sdr.radio().control(line);
                } catch (RuntimeException e) {
                    throw new ErrnoException(ErrnoException.EINVAL, e.getMessage());
                }
            }
            return length;
        }

        @Override public void close() {
            if (!partial.isEmpty()) {
                try { sdr.radio().control(partial.toString()); } catch (RuntimeException ignored) {}
            }
        }
        @Override public boolean isReadable() { return true; }
        @Override public boolean isWritable() { return true; }
        @Override public boolean isNonBlocking() { return nonBlocking; }
        @Override public void setNonBlocking(boolean nb) { nonBlocking = nb; }
    }
}
