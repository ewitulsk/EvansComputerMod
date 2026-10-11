package com.example.evanscomputermod.radio.wifi.ap;

import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;
import com.example.evanscomputermod.radio.wifi80211.MacAddress;
import com.example.evanscomputermod.radio.wifi80211.RxMeta;

import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.function.BiConsumer;

/**
 * One 802.11 radio on the {@link RadioMedium}: the endpoint the medium sees,
 * a receive queue filled off-thread, and a transmitter that turns MPDUs into
 * {@link Emission#frame} with the right rate and airtime, back to back on the
 * airtime clock (DIFS apart). Pure; the owner calls {@link #drain} and
 * {@link #send} from its own thread (the server thread for the AP block).
 */
public final class WifiAirLink implements RadioEndpoint {

    /** Frames waiting beyond this much airtime are dropped (a full transmit queue). */
    public static final long MAX_BACKLOG_US = 250_000;
    public static final int INBOX_LIMIT = 1024;

    private final UUID id;
    private final AntennaPattern antenna;
    private final double maxTxPowerDbm;
    private final ConcurrentLinkedQueue<Reception> inbox = new ConcurrentLinkedQueue<>();
    private final AtomicInteger inboxSize = new AtomicInteger();
    private final Map<MacAddress, Double> peerRssi = new ConcurrentHashMap<>();
    private volatile Pose pose;
    private volatile int wifiChannel;
    private volatile Channel channel;
    private volatile double txPowerDbm;
    private volatile boolean listening = true;
    private RadioMedium medium;
    private long nextFreeUs;
    private long txFrames, txAirtimeUs, txDropped, rxFrames, rxDropped;

    public WifiAirLink(UUID id, AntennaPattern antenna, double maxTxPowerDbm, int wifiChannel, double txPowerDbm) {
        this.id = id;
        this.antenna = antenna;
        this.maxTxPowerDbm = maxTxPowerDbm;
        this.wifiChannel = wifiChannel;
        this.channel = ApRadioPlan.channel(wifiChannel);
        this.txPowerDbm = Math.min(txPowerDbm, maxTxPowerDbm);
    }

    // ------------------------------------------------------------ control (owner thread)

    /** Joins {@code m} (leaving any previous medium); null just leaves. */
    public void attach(RadioMedium m) {
        if (m == medium) return;
        if (medium != null) medium.unregister(this);
        medium = m;
        nextFreeUs = 0;
        if (m != null && pose != null) m.register(this);
    }

    public RadioMedium medium() {
        return medium;
    }

    /** Moves the antenna; the medium notices the move itself and retraces under its rate limiter. */
    public void setPose(Pose p) {
        Pose before = pose;
        pose = p;
        if (medium != null && before == null) medium.register(this);
    }

    public void setChannel(int ch) {
        if (ch == wifiChannel) return;
        wifiChannel = ch;
        channel = ApRadioPlan.channel(ch);
        peerRssi.clear();
        if (medium != null && pose != null) medium.invalidate(this);
    }

    public void setTxPowerDbm(double dbm) {
        txPowerDbm = Math.max(-10, Math.min(maxTxPowerDbm, dbm));
    }

    public void setListening(boolean v) {
        listening = v;
    }

    public int wifiChannel() {
        return wifiChannel;
    }

    public double txPowerDbm() {
        return txPowerDbm;
    }

    /** RSSI of the last frame heard from {@code mac}, or NaN. */
    public double peerRssi(MacAddress mac) {
        Double v = peerRssi.get(mac);
        return v == null ? Double.NaN : v;
    }

    /** Put an MPDU (no FCS) on the air. Returns the emission, or null if not attached or the queue is full. */
    public Emission send(byte[] mpdu) {
        RadioMedium m = medium;
        if (m == null || pose == null || mpdu.length < 10) {
            txDropped++;
            return null;
        }
        boolean group = (mpdu[4] & 0x01) != 0;
        boolean mgmt = ((mpdu[0] >> 2) & 0x3) == 0;
        ApRadioPlan.Rate rate = group || mgmt ? ApRadioPlan.basicRate(wifiChannel)
                : ApRadioPlan.dataRate(wifiChannel, peerRssi(MacAddress.read(mpdu, 4)));
        long dur = ApRadioPlan.airtimeUs(rate, mpdu.length, wifiChannel);
        long now = m.nowMicros();
        long start = Math.max(now, nextFreeUs);
        if (start - now > MAX_BACKLOG_US) {
            txDropped++;
            return null;
        }
        nextFreeUs = start + dur + ApRadioPlan.difsUs();
        txFrames++;
        txAirtimeUs += dur;
        return m.transmit(this, Emission.frame(channel, txPowerDbm, start, dur, rate.modulation(), rate.kbps() * 1000.0,
                mpdu.clone()));
    }

    /** Hands every queued reception to {@code sink} as (MPDU, radiotap-style metadata). */
    public void drain(BiConsumer<byte[], RxMeta> sink) {
        Reception r;
        while ((r = inbox.poll()) != null) {
            inboxSize.decrementAndGet();
            Emission e = r.emission();
            byte[] f = e.payload();
            int ch = e.channel().wifiNumber();
            sink.accept(f, new RxMeta((int) Math.round(r.rssiDbm()), (int) Math.round(e.bitRate() / 1000), ch));
        }
    }

    public long txFrames() { return txFrames; }
    public long txAirtimeUs() { return txAirtimeUs; }
    public long txDropped() { return txDropped; }
    public long rxFrames() { return rxFrames; }
    public long rxDropped() { return rxDropped; }

    /** True for the modulations 802.11 radios demodulate (not controller reports, LoRa chirps...). */
    public static boolean isWifiModulation(String m) {
        return m != null && (m.startsWith("DSSS") || m.startsWith("CCK") || m.startsWith("OFDM") || m.startsWith("HT-"));
    }

    // ------------------------------------------------------------ RadioEndpoint (any thread)

    @Override public UUID id() { return id; }
    @Override public Pose pose() { return pose; }
    @Override public AntennaPattern antenna() { return antenna; }
    @Override public Channel tunedChannel() { return listening && pose != null ? channel : null; }
    @Override public double maxTxPowerDbm() { return maxTxPowerDbm; }
    @Override public double sensitivityDbm() { return -94; }

    /**
     * This radio's own address: unicast data and management frames to it are
     * acknowledged (an 802.11 ACK one SIFS after the frame), as the computers'
     * Wi-Fi modules ({@code LowMac}) expect; without it their frames to us go
     * unacknowledged and they give up after their retries. Null = never ACK.
     */
    public void setAckAddress(MacAddress own) {
        ackAddress = own == null ? null : own.bytes();
    }

    private volatile byte[] ackAddress;
    private long acksSent;

    public long acksSent() { return acksSent; }

    private void sendAck(byte[] ra, Reception r) {
        RadioMedium m = medium;
        Channel ch = channel;
        if (m == null || ch == null) return;
        byte[] ack = new byte[10];
        ack[0] = (byte) 0xd4;                           // type 1 (control), subtype 13 (ACK)
        System.arraycopy(ra, 0, ack, 4, 6);
        byte[] air = com.example.evanscomputermod.radio.wifi80211.frame.Fcs.append(ack);
        int dataKbps = r.emission().bitRate() > 0 ? (int) Math.round(r.emission().bitRate() / 1000) : 1000;
        int rate = com.example.evanscomputermod.radio.wifi.mac.WifiPhy.ackRateKbps(dataKbps, ch);
        long start = r.timestampMicros() + com.example.evanscomputermod.radio.wifi.mac.WifiPhy.sifsUs(ch);
        long dur = com.example.evanscomputermod.radio.wifi.mac.WifiPhy.airtimeUs(rate, air.length, ch);
        m.transmit(this, Emission.frame(ch, txPowerDbm, start, dur,
                com.example.evanscomputermod.radio.wifi.mac.WifiPhy.modulation(rate), rate * 1000.0, air));
        acksSent++;
    }

    @Override
    public void onReceive(Reception r) {
        Emission e = r.emission();
        byte[] f = e.payload();
        if (e.kind() != Emission.Kind.FRAME || f == null || f.length < 10 || !isWifiModulation(e.modulation())) return;
        // Radios with a real MAC (the Wi-Fi module) send a CRC-32 FCS: check and strip it, like their receivers do.
        if (f.length >= 14 && com.example.evanscomputermod.radio.wifi80211.frame.Fcs.verify(f)) {
            f = java.util.Arrays.copyOf(f, f.length - 4);
            r = new Reception(r.from(), Emission.frame(e.channel(), e.powerDbm(), e.startMicros(), e.durationMicros(),
                    e.modulation(), e.bitRate(), f), r.rssiDbm(), r.sinrDb(), r.timestampMicros());
        }
        int type = (f[0] >> 2) & 0x3;
        if (type == 1) return;                          // control frames (ACKs, ...) are the MAC's business
        byte[] own = ackAddress;
        if (own != null && f.length >= 16 && java.util.Arrays.equals(java.util.Arrays.copyOfRange(f, 4, 10), own))
            sendAck(java.util.Arrays.copyOfRange(f, 10, 16), r);
        if (f.length >= 16) peerRssi.put(MacAddress.read(f, 10), r.rssiDbm());
        if (inboxSize.incrementAndGet() > INBOX_LIMIT) {
            inboxSize.decrementAndGet();
            rxDropped++;
            return;
        }
        rxFrames++;
        inbox.add(r);
    }
}
