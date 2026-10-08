package com.example.evanscomputermod.radio.wifi;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.Reception;
import com.example.evanscomputermod.radio.medium.BasicRadioMedium;
import com.example.evanscomputermod.radio.wifi.mac.Backoff;
import com.example.evanscomputermod.radio.wifi.mac.LowMac;
import com.example.evanscomputermod.radio.wifi.mac.WifiPhy;
import com.example.evanscomputermod.radio.wifi80211.frame.Fcs;

import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.SplittableRandom;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicLong;

/** The Wi-Fi module's low MAC (FCS, airtime, backoff, ACK/retry, CSMA, RX filter) over the reference medium. */
public class LowMacTest {

    static final byte[] BCAST = {-1, -1, -1, -1, -1, -1};

    /** A radio on the medium: a LowMac behind a test endpoint. */
    static final class Radio implements RadioEndpoint {
        final UUID id = UUID.randomUUID();
        final Pose pose;
        LowMac mac;
        final List<Reception> heard = new ArrayList<>();
        Radio(double x) { pose = Pose.at("overworld", x, 64, 0); }
        @Override public UUID id() { return id; }
        @Override public Pose pose() { return pose; }
        @Override public AntennaPattern antenna() { return AntennaPattern.ISOTROPIC; }
        @Override public Channel tunedChannel() { return mac.channel(); }
        @Override public double maxTxPowerDbm() { return 20; }
        @Override public void onReceive(Reception r) {
            heard.add(r);
            mac.onReceive(r);
        }
    }

    final AtomicLong clock = new AtomicLong(5_000_000);
    final BasicRadioMedium medium = new BasicRadioMedium(clock::get, 7);

    Radio radio(double x, int last, int channel) {
        Radio r = new Radio(x);
        r.mac = new LowMac(r, () -> medium, mac(last), channel, LowMac.Options.inline(last));
        medium.register(r);
        return r;
    }

    static byte[] mac(int last) {
        return new byte[] {2, 0, 0, 0, 0, (byte) last};
    }

    /** A data frame (no FCS) from {@code sa} to {@code da} in BSS {@code bssid}. */
    static byte[] data(byte[] da, byte[] sa, byte[] bssid, int seq, int len) {
        byte[] f = new byte[24 + len];
        f[0] = 0x08;                     // data
        System.arraycopy(da, 0, f, 4, 6);
        System.arraycopy(sa, 0, f, 10, 6);
        System.arraycopy(bssid, 0, f, 16, 6);
        f[22] = (byte) (seq << 4);
        f[23] = (byte) (seq >> 4);
        for (int i = 24; i < f.length; i++) f[i] = (byte) i;
        return f;
    }

    @Test
    void airtimeFollowsPreamblePlusBitsOverRate() {
        Channel g24 = Channel.wifi24(6), g5 = Channel.wifi5(36, 20);
        // DSSS long preamble: 192 µs + 14 bytes at 1 Mb/s.
        assertEquals(304, WifiPhy.airtimeUs(1000, 14, g24));
        assertEquals(192 + 1091, WifiPhy.airtimeUs(11000, 1500, g24));
        // OFDM 54: ceil((16 + 12000 + 6) / 216) = 56 symbols -> 20 + 224 + 6 (2.4 GHz signal extension).
        assertEquals(250, WifiPhy.airtimeUs(54000, 1500, g24));
        assertEquals(244, WifiPhy.airtimeUs(54000, 1500, g5));
        // HT MCS7 (65 Mb/s): ceil(12022 / 260) = 47 symbols, 36 µs HT-mixed preamble.
        assertEquals(36 + 188 + 6, WifiPhy.airtimeUs(65000, 1500, g24));
        // OFDM 6 Mb/s ACK: ceil(134 / 24) = 6 symbols.
        assertEquals(20 + 24 + 6, WifiPhy.airtimeUs(6000, 14, g24));
        assertEquals("OFDM-54", WifiPhy.modulation(54000));
        assertEquals("HT-MCS7", WifiPhy.modulation(65000));
        assertEquals("CCK-5.5", WifiPhy.modulation(5500));
        assertEquals("DSSS-1", WifiPhy.modulation(1234), "unknown rates fall back to 1 Mb/s");
        assertEquals(28, WifiPhy.difsUs(g24));
        assertEquals(34, WifiPhy.difsUs(g5));
        assertNull(WifiPhy.channel(14));
        assertEquals(5.180e9, WifiPhy.channel(36).centerHz(), 1);
    }

    @Test
    void backoffWindowDoublesToCwMaxAndStopsAfterRetryLimit() {
        Backoff b = new Backoff(new SplittableRandom(1));
        int[] expect = {15, 31, 63, 127, 255, 511, 1023, 1023};
        for (int i = 0; i < 8; i++) {
            assertEquals(expect[i], b.cw(), "cw before attempt " + (i + 1));
            int s = b.drawSlots();
            assertTrue(s >= 0 && s <= b.cw());
            boolean more = b.onFailure();
            assertEquals(i < 7, more, "1 + 7 retries");
        }
        b.reset();
        assertEquals(15, b.cw());
        assertEquals(0, b.attempts());
    }

    @Test
    void framesGoOnTheAirWithFcsAndArriveWithoutIt() {
        Radio a = radio(0, 1, 6), b = radio(10, 2, 6);
        byte[] f = data(BCAST, mac(1), mac(1), 1, 40);
        assertEquals(0, a.mac.submit(f, 54000, 200));
        Reception r = b.heard.get(0);
        assertEquals(f.length + 4, r.payload().length);
        assertTrue(Fcs.verify(r.payload()));
        assertEquals("OFDM-54", r.emission().modulation());
        assertEquals(WifiPhy.airtimeUs(54000, f.length + 4, a.mac.channel()), r.emission().durationMicros());
        LowMac.RxFrame got = b.mac.poll();
        assertArrayEquals(f, got.frame());
        assertEquals(54000, got.rateKbps());
        assertEquals(6, got.channel());
        assertTrue(got.rssiDbmX10() < -300 && got.rssiDbmX10() > -700, "rssi " + got.rssiDbmX10());
        assertEquals(r.timestampMicros(), got.timestampUs());
        // A transmitter that sends no FCS (another implementation) is still understood.
        Radio raw = radio(5, 3, 6);
        medium.transmit(raw, Emission.frame(Channel.wifi24(6), 15, clock.get(), 100, "OFDM-6", 6e6, f));
        assertArrayEquals(f, b.mac.poll().frame());
    }

    @Test
    void unicastIsAckedAfterSifsAndReportedOnce() {
        Radio a = radio(0, 1, 6), b = radio(10, 2, 6);
        byte[] f = data(mac(2), mac(1), mac(1), 9, 100);
        a.mac.submit(f, 24000, 200);
        LowMac.TxStatus st = a.mac.pollStatus();
        assertTrue(st.acked());
        assertEquals(1, st.attempts());
        assertEquals(24000, st.rateKbps());
        assertEquals(9 << 4, st.seqCtrl());
        assertEquals(0x08, st.frameControl());
        assertNull(a.mac.pollStatus());
        assertEquals(1, b.mac.acksSent.get());
        assertEquals(1, a.mac.acksReceived.get());
        // The ACK: 14 bytes to A, one SIFS after the data frame ended, at a basic OFDM rate.
        Reception dataAtB = b.heard.get(0);
        Reception ack = a.heard.stream().filter(x -> x.payload().length == 14).findFirst().orElseThrow();
        byte[] ab = ack.payload();
        assertEquals((byte) 0xd4, ab[0]);
        assertArrayEquals(mac(1), Arrays.copyOfRange(ab, 4, 10));
        assertEquals(dataAtB.timestampMicros() + 10, ack.emission().startMicros());
        assertEquals("OFDM-6", ack.emission().modulation());
        // Control frames never reach the host in normal mode.
        assertNull(a.mac.poll());
    }

    @Test
    void unackedUnicastIsRetriedSevenTimesWithRetryBitAndBackoff() {
        Radio a = radio(0, 1, 6);
        Radio sniffer = radio(5, 9, 6);
        sniffer.mac.setRxFilter(LowMac.MODE_MONITOR, null);
        byte[] f = data(mac(0x77), mac(1), mac(1), 3, 60);   // nobody has this address
        a.mac.submit(f, 6000, 200);
        LowMac.TxStatus st = a.mac.pollStatus();
        assertFalse(st.acked());
        assertEquals(8, st.attempts());
        assertEquals(1, a.mac.txFailed.get());
        List<LowMac.RxFrame> seen = new ArrayList<>();
        for (LowMac.RxFrame x; (x = sniffer.mac.poll()) != null; ) seen.add(x);
        assertEquals(8, seen.size());
        assertEquals(0, seen.get(0).frame()[1] & 0x08, "first attempt has no Retry bit");
        for (int i = 1; i < 8; i++) assertEquals(0x08, seen.get(i).frame()[1] & 0x08, "retry " + i);
        // Attempts are spaced by at least DIFS after the previous frame ended (backoff on top).
        List<Reception> air = sniffer.heard;
        for (int i = 1; i < air.size(); i++) {
            long prevEnd = air.get(i - 1).timestampMicros();
            long start = air.get(i).emission().startMicros();
            assertTrue(start >= prevEnd + WifiPhy.difsUs(a.mac.channel()), "attempt " + i);
        }
        assertEquals(0, sniffer.mac.acksSent.get(), "monitor mode never ACKs");
    }

    @Test
    void groupFramesAreNotAckedAndReportSuccess() {
        Radio a = radio(0, 1, 6), b = radio(10, 2, 6);
        a.mac.submit(data(BCAST, mac(1), mac(1), 4, 20), 1000, 200);
        LowMac.TxStatus st = a.mac.pollStatus();
        assertTrue(st.acked());
        assertEquals(1, st.attempts());
        assertEquals(0, b.mac.acksSent.get());
        assertNotNull(b.mac.poll());
    }

    @Test
    void rxFilterModes() {
        Radio a = radio(0, 1, 6), b = radio(10, 2, 6), c3 = radio(-10, 3, 6);
        byte[] bss = mac(0x40), other = mac(0x41);
        // Normal + BSSID: own unicast and group from our BSS only.
        b.mac.setRxFilter(LowMac.MODE_NORMAL, bss);
        a.mac.submit(data(mac(3), mac(1), bss, 1, 10), 6000, 100);       // someone else's unicast
        a.mac.submit(data(BCAST, mac(1), other, 2, 10), 6000, 100);      // group, other BSS
        a.mac.submit(data(BCAST, mac(1), bss, 3, 10), 6000, 100);        // group, our BSS
        a.mac.submit(data(mac(2), mac(1), other, 4, 10), 6000, 100);     // to us
        assertEquals(3, seq(b.mac.poll()));
        assertEquals(4, seq(b.mac.poll()));
        assertNull(b.mac.poll());
        // Promiscuous: every data frame, but still no control frames.
        b.mac.setRxFilter(LowMac.MODE_PROMISC, null);
        a.mac.submit(data(mac(3), mac(1), bss, 5, 10), 6000, 100);
        a.mac.submit(data(mac(2), mac(1), bss, 6, 10), 6000, 100);       // acked by b
        assertEquals(5, seq(b.mac.poll()));
        assertEquals(6, seq(b.mac.poll()));
        assertNull(b.mac.poll());
        // Monitor: everything incl. the ACK that a third radio sends.
        Radio c = radio(20, 4, 6);
        assertNotNull(c3.mac.poll(), "the third radio got its unicast");
        b.mac.setRxFilter(LowMac.MODE_MONITOR, null);
        a.mac.submit(data(mac(4), mac(1), bss, 7, 10), 6000, 100);
        assertEquals(7, seq(b.mac.poll()));
        LowMac.RxFrame ack = b.mac.poll();
        assertEquals((byte) 0xd4, ack.frame()[0]);
        assertEquals(10, ack.frame().length, "FCS stripped from the ACK too");
        assertTrue(c.mac.poll() != null);
        assertFalse(b.mac.setRxFilter(3, null));
    }

    @Test
    void otherChannelsAreNotHeardAndRetuningInvalidates() {
        Radio a = radio(0, 1, 1), b = radio(10, 2, 11);
        a.mac.submit(data(mac(2), mac(1), mac(1), 1, 10), 6000, 200);
        assertFalse(a.mac.pollStatus().acked(), "1 and 11 don't overlap");
        assertTrue(b.mac.setChannel(1));
        assertFalse(b.mac.setChannel(14), "channel 14 is not in the 2.4 GHz plan");
        a.mac.submit(data(mac(2), mac(1), mac(1), 2, 10), 6000, 200);
        assertTrue(a.mac.pollStatus().acked());
    }

    @Test
    void carrierSenseDefersOnABusyChannelAndNavFromHeardFrames() {
        Radio a = radio(0, 1, 6), b = radio(10, 2, 6);
        Radio jammer = radio(3, 5, 6);
        // An energy burst covering "now" makes the channel busy for A.
        medium.transmit(jammer, Emission.energy(Channel.wifi24(6), 20, clock.get(), 50_000));
        a.mac.submit(data(BCAST, mac(1), mac(1), 1, 10), 6000, 200);
        assertEquals(1, a.mac.ccaDeferrals.get());
        // Virtual carrier sense: after hearing B's long frame, A starts no earlier than its end + DIFS.
        clock.addAndGet(100_000);
        b.mac.submit(data(BCAST, mac(2), mac(2), 2, 1400), 1000, 200);   // ~11.5 ms at 1 Mb/s
        long bEnd = a.heard.get(a.heard.size() - 1).timestampMicros();
        a.mac.submit(data(BCAST, mac(1), mac(1), 3, 10), 6000, 200);
        Reception aAtB = b.heard.get(b.heard.size() - 1);
        assertTrue(aAtB.emission().startMicros() >= bEnd + WifiPhy.difsUs(a.mac.channel()),
                "start " + aAtB.emission().startMicros() + " vs busy until " + bEnd);
    }

    @Test
    void submitValidatesAndCapsPower() {
        Radio a = radio(0, 1, 6), b = radio(10, 2, 6);
        assertEquals(-1, a.mac.submit(new byte[5], 1000, 100));
        a.mac.setMaxPowerDbm(5);
        a.mac.submit(data(BCAST, mac(1), mac(1), 1, 10), 1000, 200);
        assertEquals(5, b.heard.get(0).emission().powerDbm(), 1e-9);
        a.mac.setMaxPowerDbm(99);
        assertEquals(LowMac.MAX_TX_POWER_DBM, a.mac.maxPowerDbm());
    }

    static int seq(LowMac.RxFrame f) {
        assertNotNull(f);
        return ((f.frame()[22] & 0xff) | (f.frame()[23] & 0xff) << 8) >> 4;
    }
}
