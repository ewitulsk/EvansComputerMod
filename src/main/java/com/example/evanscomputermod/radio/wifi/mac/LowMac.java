package com.example.evanscomputermod.radio.wifi.mac;

import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;
import com.example.evanscomputermod.radio.wifi80211.frame.Fcs;

import java.util.Arrays;
import java.util.SplittableRandom;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.BlockingQueue;
import java.util.concurrent.Executor;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.Supplier;

/**
 * The SoftMAC "firmware" of a Wi-Fi radio: only the timing-critical part of
 * 802.11, the rest lives in the kernel (ecm-wifi). Pure Java against the
 * {@link RadioMedium} API, so it runs the same in the world and in JUnit.
 *
 * <ul>
 *   <li><b>FCS:</b> frames from the host carry none; the MAC appends the CRC-32
 *       before they go on the air and strips/verifies it on receive. Payloads
 *       without a valid FCS are accepted as FCS-less MPDUs (other transmitters).</li>
 *   <li><b>ACK:</b> a unicast data or management frame addressed to this radio is
 *       acknowledged with a 14-byte ACK one SIFS after the frame ends, from inside
 *       the medium's delivery callback (not in monitor mode).</li>
 *   <li><b>Retransmission:</b> an unacknowledged unicast frame is retried up to
 *       {@link WifiPhy#RETRY_LIMIT} times with the Retry bit set; each attempt
 *       draws a new binary-exponential backoff ({@link Backoff}).</li>
 *   <li><b>CSMA/CA:</b> physical carrier sense ({@link RadioMedium#channelPowerDbm}
 *       above {@link #CS_THRESHOLD_DBM}) defers, virtual carrier sense keeps the
 *       end of every frame heard (plus its Duration field) as a NAV; an attempt
 *       starts DIFS + backoff slots after the medium (and our own last frame) is
 *       free, on the medium's airtime clock.</li>
 *   <li><b>Airtime:</b> preamble + bits / rate ({@link WifiPhy#airtimeUs}); with
 *       pacing on, the transmit thread waits for the airtime clock so a radio
 *       cannot exceed its rate.</li>
 *   <li><b>RX filter:</b> {@link #MODE_NORMAL} (own address + group, group only
 *       from the BSSID when one is set), {@link #MODE_PROMISC} (every data and
 *       management frame) and {@link #MODE_MONITOR} (everything incl. control
 *       frames; no ACKs sent).</li>
 *   <li><b>Queues:</b> the medium calls {@link #onReceive} off-thread; frames and
 *       per-frame transmit statuses queue here until the host drains them, and
 *       {@code rxNotify} runs (the computer's Wi-Fi IRQ).</li>
 * </ul>
 */
public final class LowMac {

    public static final int MODE_NORMAL = 0;
    public static final int MODE_PROMISC = 1;
    public static final int MODE_MONITOR = 2;

    /** Energy-detect threshold for "medium busy" (the 802.11 CCA level for 20 MHz). */
    public static final double CS_THRESHOLD_DBM = -82;
    public static final double MAX_TX_POWER_DBM = 20;
    public static final int TX_QUEUE = 64;
    public static final int RX_QUEUE = 256;
    public static final int STATUS_QUEUE = 256;

    /** A received frame (no FCS) with radiotap-style metadata. */
    public record RxFrame(byte[] frame, int rssiDbmX10, int rateKbps, int channel, long timestampUs, boolean fcsOk) {}

    /** What happened to one transmitted frame. */
    public record TxStatus(boolean acked, int attempts, int rateKbps, int seqCtrl, int frameControl) {}

    /** Knobs (tests run inline and without real-time waits). */
    public record Options(Executor executor, boolean pace, long ackGraceNanos, long csMaxWaitNanos, long seed) {
        public static Options world(long seed) {
            return new Options(SHARED, true, 5_000_000L, 10_000_000L, seed);
        }

        public static Options inline(long seed) {
            return new Options(Runnable::run, false, 0, 0, seed);
        }
    }

    private static final Executor SHARED = Executors.newCachedThreadPool(r -> {
        Thread t = new Thread(r, "ecm-wifi-mac");
        t.setDaemon(true);
        return t;
    });

    private record TxReq(byte[] frame, int rateKbps, double powerDbm) {}

    private final RadioEndpoint self;
    private final Supplier<RadioMedium> medium;
    private final Options opt;
    private final Backoff backoff;
    private volatile byte[] mac;
    private volatile Channel channel;
    private volatile int channelNumber;
    private volatile int mode = MODE_NORMAL;
    private volatile byte[] bssid;
    private volatile double maxPowerDbm = MAX_TX_POWER_DBM;
    private volatile Runnable rxNotify = () -> {};

    private final BlockingQueue<TxReq> txq = new ArrayBlockingQueue<>(TX_QUEUE);
    private final BlockingQueue<RxFrame> rxq = new ArrayBlockingQueue<>(RX_QUEUE);
    private final BlockingQueue<TxStatus> statusq = new ArrayBlockingQueue<>(STATUS_QUEUE);
    private final AtomicBoolean draining = new AtomicBoolean();
    private final AtomicLong nav = new AtomicLong();
    private volatile long lastTxEnd;

    // The one frame waiting for an ACK.
    private volatile byte[] ackFrom;
    private final AtomicBoolean acked = new AtomicBoolean();

    // Counters (peripheral stats, tests).
    public final AtomicLong txFrames = new AtomicLong(), txAttempts = new AtomicLong(), txFailed = new AtomicLong(),
            rxFrames = new AtomicLong(), rxDropped = new AtomicLong(), acksSent = new AtomicLong(),
            acksReceived = new AtomicLong(), ccaDeferrals = new AtomicLong(), airtimeUs = new AtomicLong();

    public LowMac(RadioEndpoint self, Supplier<RadioMedium> medium, byte[] mac, int channelNumber, Options opt) {
        this.self = self;
        this.medium = medium;
        this.opt = opt;
        this.backoff = new Backoff(new SplittableRandom(opt.seed()));
        this.mac = mac.clone();
        if (!setChannel(channelNumber)) setChannel(1);
    }

    // ------------------------------------------------------------ configuration

    public byte[] mac() {
        return mac.clone();
    }

    public Channel channel() {
        return channel;
    }

    public int channelNumber() {
        return channelNumber;
    }

    public int mode() {
        return mode;
    }

    public void setRxNotify(Runnable r) {
        rxNotify = r == null ? () -> {} : r;
    }

    public void setMaxPowerDbm(double dbm) {
        maxPowerDbm = Math.max(0, Math.min(MAX_TX_POWER_DBM, dbm));
    }

    public double maxPowerDbm() {
        return maxPowerDbm;
    }

    /** Tune; returns false for an invalid channel number. */
    public boolean setChannel(int number) {
        Channel c = WifiPhy.channel(number);
        if (c == null) return false;
        channelNumber = number;
        channel = c;
        RadioMedium m = medium.get();
        if (m != null) m.invalidate(self);
        return true;
    }

    /** RX filter; {@code bssid} (6 bytes) or null. Returns false for a bad mode. */
    public boolean setRxFilter(int mode, byte[] bssid) {
        if (mode < MODE_NORMAL || mode > MODE_MONITOR) return false;
        this.mode = mode;
        this.bssid = bssid == null || bssid.length != 6 || isZero(bssid) ? null : bssid.clone();
        return true;
    }

    // ------------------------------------------------------------ host side

    /**
     * Queue a frame (no FCS) for transmission. Returns 0, -1 for bad arguments,
     * -2 when the queue is full.
     */
    public int submit(byte[] frame, int rateKbps, int powerDbmX10) {
        if (frame == null || frame.length < 10 || frame.length > 2346) return -1;
        int rate = WifiPhy.valid(rateKbps) ? rateKbps : 1000;
        double p = Math.min(maxPowerDbm, powerDbmX10 / 10.0);
        if (!txq.offer(new TxReq(frame.clone(), rate, p))) return -2;
        kick();
        return 0;
    }

    public RxFrame poll() {
        return rxq.poll();
    }

    public TxStatus pollStatus() {
        return statusq.poll();
    }

    public int rxQueued() {
        return rxq.size();
    }

    private void kick() {
        if (draining.compareAndSet(false, true)) {
            opt.executor().execute(this::drain);
        }
    }

    private void drain() {
        try {
            TxReq r;
            while ((r = txq.poll()) != null) {
                transmit(r);
            }
        } finally {
            draining.set(false);
        }
        // A frame queued between the last poll and clearing the flag.
        if (!txq.isEmpty()) kick();
    }

    // ------------------------------------------------------------ transmit path

    private void transmit(TxReq r) {
        RadioMedium m = medium.get();
        byte[] f = r.frame;
        int fc = (f[0] & 0xff) | (f[1] & 0xff) << 8;
        int seqCtrl = f.length >= 24 && type(f) != 1 ? (f[22] & 0xff) | (f[23] & 0xff) << 8 : 0;
        if (m == null) {
            pushStatus(new TxStatus(false, 1, r.rateKbps, seqCtrl, fc));
            return;
        }
        boolean unicast = type(f) != 1 && (f[4] & 1) == 0;
        byte[] ra = Arrays.copyOfRange(f, 4, 10);
        Channel ch = channel;
        backoff.reset();
        boolean ok = false;
        int attempts = 0;
        while (true) {
            byte[] air = Fcs.append(attempts > 0 ? withRetryBit(f) : f);
            waitForClearChannel(m, ch);
            int slots = backoff.drawSlots();
            attempts = backoff.attempts();
            long now = m.nowMicros();
            long start = Math.max(Math.max(now, nav.get()), lastTxEnd) + WifiPhy.difsUs(ch) + (long) slots * WifiPhy.SLOT_US;
            long dur = WifiPhy.airtimeUs(r.rateKbps, air.length, ch);
            pace(m, start);
            if (unicast) {
                acked.set(false);
                ackFrom = ra;
            }
            m.transmit(self, Emission.frame(ch, r.powerDbm, start, dur, WifiPhy.modulation(r.rateKbps), r.rateKbps * 1000.0, air));
            lastTxEnd = start + dur;
            txAttempts.incrementAndGet();
            airtimeUs.addAndGet(dur);
            if (!unicast) {
                ok = true;
                break;
            }
            if (awaitAck()) {
                ok = true;
                acksReceived.incrementAndGet();
                // The ACK occupies the medium too.
                lastTxEnd = start + dur + WifiPhy.sifsUs(ch) + WifiPhy.airtimeUs(WifiPhy.ackRateKbps(r.rateKbps, ch), WifiPhy.ACK_LEN, ch);
                break;
            }
            if (!backoff.onFailure() || attempts > WifiPhy.RETRY_LIMIT) break;
        }
        ackFrom = null;
        backoff.reset();
        txFrames.incrementAndGet();
        if (!ok) txFailed.incrementAndGet();
        pushStatus(new TxStatus(ok, attempts, r.rateKbps, seqCtrl, fc));
    }

    private boolean awaitAck() {
        if (acked.get()) return true;
        long until = System.nanoTime() + opt.ackGraceNanos();
        while (System.nanoTime() < until) {
            if (acked.get()) return true;
            Thread.onSpinWait();
            if (opt.ackGraceNanos() > 1_000_000) {
                try {
                    Thread.sleep(0, 200_000);
                } catch (InterruptedException e) {
                    Thread.currentThread().interrupt();
                    return acked.get();
                }
            }
        }
        return acked.get();
    }

    private void waitForClearChannel(RadioMedium m, Channel ch) {
        if (m.channelPowerDbm(self, ch) < CS_THRESHOLD_DBM) return;
        ccaDeferrals.incrementAndGet();
        long until = System.nanoTime() + opt.csMaxWaitNanos();
        while (System.nanoTime() < until && m.channelPowerDbm(self, ch) >= CS_THRESHOLD_DBM) {
            try {
                Thread.sleep(0, 250_000);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                return;
            }
        }
    }

    /** Wait (bounded) until the airtime clock reaches {@code startUs}. */
    private void pace(RadioMedium m, long startUs) {
        if (!opt.pace()) return;
        long deadline = System.nanoTime() + 200_000_000L;
        while (m.nowMicros() < startUs - 1000 && System.nanoTime() < deadline) {
            try {
                Thread.sleep(1);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                return;
            }
        }
    }

    private void pushStatus(TxStatus s) {
        if (!statusq.offer(s)) {
            statusq.poll();
            statusq.offer(s);
        }
        rxNotify.run();
    }

    // ------------------------------------------------------------ receive path

    /** A frame heard on the medium (called off-thread by the medium). */
    public void onReceive(Reception r) {
        byte[] payload = r.payload();
        if (payload == null || payload.length < 10) return;
        boolean fcs = payload.length >= 14 && Fcs.verify(payload);
        byte[] f = fcs ? Arrays.copyOf(payload, payload.length - 4) : payload;
        int t = type(f), st = subtype(f);
        // Virtual carrier sense: the medium is reserved until the frame's end + its Duration.
        int durField = (f[2] & 0xff) | (f[3] & 0x7f) << 8;
        long end = r.timestampMicros() + ((f[3] & 0x80) == 0 ? durField : 0);
        nav.accumulateAndGet(end, Math::max);

        byte[] a1 = Arrays.copyOfRange(f, 4, 10);
        boolean toMe = Arrays.equals(a1, mac);
        int m = mode;
        if (t == 1 && st == 13) {                       // ACK
            byte[] expect = ackFrom;
            if (toMe && expect != null) acked.set(true);
            if (m == MODE_MONITOR) deliver(f, r);
            return;
        }
        if (toMe && m != MODE_MONITOR && t != 1 && f.length >= 16) {
            sendAck(Arrays.copyOfRange(f, 10, 16), r);
        }
        boolean accept = switch (m) {
            case MODE_MONITOR -> true;
            case MODE_PROMISC -> t != 1;
            default -> t != 1 && (toMe || ((a1[0] & 1) != 0 && fromBss(f)));
        };
        if (accept) deliver(f, r);
    }

    private boolean fromBss(byte[] f) {
        byte[] b = bssid;
        if (b == null || f.length < 22) return true;
        return regionEquals(f, 10, b) || regionEquals(f, 16, b);
    }

    private void sendAck(byte[] ra, Reception r) {
        RadioMedium m = medium.get();
        if (m == null) return;
        Channel ch = channel;
        byte[] ack = new byte[10];
        ack[0] = (byte) 0xd4;                           // type 1 (control), subtype 13 (ACK)
        System.arraycopy(ra, 0, ack, 4, 6);
        byte[] air = Fcs.append(ack);
        int dataRate = r.emission().bitRate() > 0 ? (int) Math.round(r.emission().bitRate() / 1000) : 1000;
        int rate = WifiPhy.ackRateKbps(dataRate, ch);
        long start = r.timestampMicros() + WifiPhy.sifsUs(ch);
        long dur = WifiPhy.airtimeUs(rate, air.length, ch);
        m.transmit(self, Emission.frame(ch, maxPowerDbm, start, dur, WifiPhy.modulation(rate), rate * 1000.0, air));
        acksSent.incrementAndGet();
        airtimeUs.addAndGet(dur);
    }

    private void deliver(byte[] f, Reception r) {
        int rate = r.emission().bitRate() > 0 ? (int) Math.round(r.emission().bitRate() / 1000) : 1000;
        RxFrame rx = new RxFrame(f, (int) Math.round(r.rssiDbm() * 10), rate, channelNumber, r.timestampMicros(), true);
        if (rxq.offer(rx)) {
            rxFrames.incrementAndGet();
        } else {
            rxDropped.incrementAndGet();
        }
        rxNotify.run();
    }

    // ------------------------------------------------------------ frame helpers

    static int type(byte[] f) {
        return (f[0] >> 2) & 3;
    }

    static int subtype(byte[] f) {
        return (f[0] >> 4) & 15;
    }

    static byte[] withRetryBit(byte[] f) {
        byte[] c = f.clone();
        c[1] |= 0x08;
        return c;
    }

    private static boolean isZero(byte[] b) {
        for (byte x : b) if (x != 0) return false;
        return true;
    }

    private static boolean regionEquals(byte[] f, int off, byte[] b) {
        for (int i = 0; i < 6; i++) if (f[off + i] != b[i]) return false;
        return true;
    }
}
