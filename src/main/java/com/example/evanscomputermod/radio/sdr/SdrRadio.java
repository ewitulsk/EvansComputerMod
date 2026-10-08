package com.example.evanscomputermod.radio.sdr;

import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;

import java.util.ArrayList;
import java.util.List;
import java.util.function.Consumer;
import java.util.function.IntSupplier;
import java.util.function.LongSupplier;

/**
 * The radio half of an SDR, independent of Minecraft: tuning, gain/AGC,
 * receive on the world sample clock and IQ transmit through the medium.
 *
 * <p>Receive is lazy: samples are synthesised when a program reads them, from
 * the last read up to "now" on the world clock, so idle SDRs cost nothing. A
 * reader that falls more than {@link #RX_BUFFER_SECONDS} behind loses samples
 * and gets an {@code sdr_overflow} event. Transmit schedules written samples
 * back to back; a writer that falls behind the clock leaves silence and gets
 * {@code sdr_underflow}.
 */
public final class SdrRadio {

    public static final double RX_BUFFER_SECONDS = 0.25;
    public enum Format { CS16, CF32 }

    private final SdrTier tier;
    private final RadioEndpoint endpoint;
    private final LongSupplier clockMicros;
    private final IntSupplier rateCap;
    private final Consumer<String> events;
    private final long seed;

    private volatile double centerHz = 100e6;
    private volatile int sampleRate = 48_000;
    private volatile double bandwidthHz = 0;    // 0 = sample rate
    private volatile double gainDb = 30;
    private volatile boolean agc = true;
    private volatile boolean txEnabled;
    private volatile double txPowerDbm;
    private volatile Format format = Format.CS16;

    private long rxCursor = Long.MIN_VALUE;     // next absolute sample to deliver
    private long txCursorMicros = Long.MIN_VALUE;
    private long overflows, underflows, samplesRead, samplesWritten;
    /** Vetoes a transmission (server rules, claims); null = always allowed. */
    private volatile java.util.function.Predicate<Emission> txGate;

    public void setTxGate(java.util.function.Predicate<Emission> gate) {
        txGate = gate;
    }

    public SdrRadio(SdrTier tier, RadioEndpoint endpoint, LongSupplier clockMicros, IntSupplier rateCap,
                    Consumer<String> events, long seed) {
        this.tier = tier;
        this.endpoint = endpoint;
        this.clockMicros = clockMicros;
        this.rateCap = rateCap;
        this.events = events;
        this.seed = seed;
        this.sampleRate = Math.min(48_000, maxRate());
        this.txPowerDbm = tier.maxTxDbm;
    }

    public SdrTier tier() { return tier; }
    public double centerHz() { return centerHz; }
    public int sampleRate() { return sampleRate; }
    public double gainDb() { return gainDb; }
    public boolean agc() { return agc; }
    public boolean txEnabled() { return txEnabled; }
    public Format format() { return format; }
    public long overflows() { return overflows; }
    public long underflows() { return underflows; }

    public int maxRate() {
        return Math.min(tier.maxRate, rateCap.getAsInt());
    }

    /** The channel the receiver listens on (centre ± sample rate / 2). */
    public Channel channel() {
        return new Channel(centerHz, bandwidthHz > 0 ? bandwidthHz : sampleRate);
    }

    public synchronized void setFrequency(double hz) {
        if (!(hz >= tier.minHz && hz <= tier.maxHz))
            throw new IllegalArgumentException(String.format("%s tunes %.0f Hz - %.0f Hz, got %.0f", tier.id(), tier.minHz, tier.maxHz, hz));
        centerHz = hz;
    }

    public synchronized void setSampleRate(int rate) {
        int max = maxRate();
        if (rate < 1000 || rate > max) throw new IllegalArgumentException("sample rate must be 1000-" + max + ", got " + rate);
        sampleRate = rate;
        rxCursor = Long.MIN_VALUE;
    }

    public void setBandwidth(double hz) {
        if (hz < 0) throw new IllegalArgumentException("bandwidth must be >= 0");
        bandwidthHz = hz;
    }

    public void setGain(double db) {
        if (db < 0 || db > 60) throw new IllegalArgumentException("gain must be 0-60 dB, got " + db);
        gainDb = db;
        agc = false;
    }

    public void setAgc(boolean on) {
        agc = on;
    }

    public void setFormat(Format f) {
        format = f;
    }

    public synchronized void setTx(boolean on, double powerDbm) {
        if (on && !tier.canTransmit) throw new IllegalArgumentException(tier.id() + " is receive-only");
        txEnabled = on;
        txPowerDbm = Math.min(tier.maxTxDbm, powerDbm);
        txCursorMicros = Long.MIN_VALUE;
    }

    /** The current absolute sample index on the world sample clock (for syncing rx and tx). */
    public long timestamp() {
        return IqSynthesizer.sampleAt(clockMicros.getAsLong(), sampleRate);
    }

    /** Samples available to read right now without blocking. */
    public synchronized long available() {
        long now = timestamp();
        if (rxCursor == Long.MIN_VALUE) return 0;
        return Math.max(0, now - rxCursor);
    }

    /**
     * Read up to {@code maxSamples} complex samples (interleaved floats into
     * {@code out}); returns how many. Never blocks: returns 0 when the clock
     * hasn't advanced.
     */
    public synchronized int read(RadioMedium medium, float[] out, int maxSamples) {
        int rate = sampleRate;
        long now = IqSynthesizer.sampleAt(clockMicros.getAsLong(), rate);
        if (rxCursor == Long.MIN_VALUE) rxCursor = now;
        long behind = now - rxCursor;
        long cap = (long) (RX_BUFFER_SECONDS * rate);
        if (behind > cap) {
            overflows++;
            events.accept("sdr_overflow");
            rxCursor = now - cap;
            behind = cap;
        }
        int n = (int) Math.min(behind, maxSamples);
        if (n <= 0) return 0;
        long from = (long) IqSynthesizer.microsAt(rxCursor, rate), to = (long) Math.ceil(IqSynthesizer.microsAt(rxCursor + n, rate));
        List<RadioMedium.Heard> heard = new ArrayList<>();
        if (medium != null) medium.forEachHeard(endpoint, channel(), from - 2000, to, heard::add);
        IqSynthesizer synth = new IqSynthesizer(rate, centerHz, tier.adcBits, endpoint.noiseFigureDb(), seed);
        double peak = synth.synthesize(rxCursor, n, heard, gainDb, out);
        if (agc && peak > 0) {
            // Aim the peak at -6 dBFS, moving at most 6 dB per block.
            double adjust = Math.max(-6, Math.min(6, 20 * Math.log10(0.5 / peak)));
            gainDb = Math.max(0, Math.min(60, gainDb + adjust));
        }
        rxCursor += n;
        samplesRead += n;
        return n;
    }

    /**
     * Transmit {@code n} interleaved complex samples (RMS 1 = transmit power).
     * Returns the absolute start sample used.
     */
    public synchronized long write(RadioMedium medium, float[] iq, int n) {
        if (!txEnabled) throw new IllegalStateException("transmit is off (tx_enable first)");
        if (medium == null || n <= 0) return -1;
        long now = clockMicros.getAsLong();
        if (txCursorMicros == Long.MIN_VALUE) txCursorMicros = now;
        if (txCursorMicros < now) {
            underflows++;
            events.accept("sdr_underflow");
            txCursorMicros = now;
        }
        float[] chunk = java.util.Arrays.copyOf(iq, 2 * n);
        Emission e = Emission.iq(new Channel(centerHz, sampleRate), txPowerDbm, txCursorMicros, chunk, sampleRate);
        var gate = txGate;
        if (gate != null && !gate.test(e)) throw new IllegalStateException("transmission blocked");
        Emission sent = medium.transmit(endpoint, e);
        long start = IqSynthesizer.sampleAt(sent.startMicros(), sampleRate);
        txCursorMicros = sent.endMicros();
        samplesWritten += n;
        return start;
    }

    /** {@code key value} lines for /dev/sdrctl and stats(). */
    public String describe() {
        return "tier " + tier.id() + "\nfreq " + (long) centerHz + "\nrate " + sampleRate + "\nmax_rate " + maxRate()
                + "\nbw " + (long) channel().bandwidthHz() + "\ngain " + String.format("%.1f", gainDb) + "\nagc " + (agc ? 1 : 0)
                + "\nformat " + format.name().toLowerCase() + "\ntx " + (txEnabled ? 1 : 0) + "\ntx_power_dbm " + txPowerDbm
                + "\nadc_bits " + tier.adcBits + "\ntimestamp " + timestamp() + "\nread " + samplesRead
                + "\nwritten " + samplesWritten + "\noverflows " + overflows + "\nunderflows " + underflows + "\n";
    }

    /** Apply one /dev/sdrctl command line. */
    public void control(String line) {
        String[] p = line.trim().split("\\s+");
        if (p.length == 0 || p[0].isEmpty()) return;
        switch (p[0]) {
            case "freq" -> setFrequency(Double.parseDouble(p[1]));
            case "rate" -> setSampleRate(Integer.parseInt(p[1]));
            case "bw" -> setBandwidth(Double.parseDouble(p[1]));
            case "gain" -> {
                if (p[1].equals("agc")) setAgc(true);
                else setGain(Double.parseDouble(p[1]));
            }
            case "agc" -> setAgc(!p[1].equals("0") && !p[1].equals("off"));
            case "format" -> setFormat(p[1].equalsIgnoreCase("cf32") ? Format.CF32 : Format.CS16);
            case "tx" -> setTx(p[1].equals("on") || p[1].equals("1"), p.length > 2 ? Double.parseDouble(p[2]) : tier.maxTxDbm);
            default -> throw new IllegalArgumentException("unknown sdrctl command: " + p[0]);
        }
    }
}
