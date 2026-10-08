package com.example.evanscomputermod.radio.wifi.ap;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.radio.api.Band;
import com.example.evanscomputermod.radio.wifi80211.MacAddress;
import com.example.evanscomputermod.radio.wifi80211.Security;
import com.example.evanscomputermod.radio.wifi80211.ap.ApConfig;

import org.junit.jupiter.api.Test;

import java.util.List;
import java.util.Map;

/** Rates, airtime, antenna, settings validation and the IPv4 responder of the Access Point lane. */
public class AccessPointHelpersTest {

    static final MacAddress BSSID = MacAddress.parse("02:a1:12:34:56:78");

    // ------------------------------------------------------------ rates / airtime

    @Test
    void basicRatesFollowTheBand() {
        assertEquals("DSSS-1", ApRadioPlan.basicRate(6).modulation());
        assertEquals("OFDM-6", ApRadioPlan.basicRate(36).modulation());
        assertEquals(Band.WIFI_2G4, ApRadioPlan.channel(11).band());
        assertEquals(Band.WIFI_5G, ApRadioPlan.channel(149).band());
        assertEquals(20e6, ApRadioPlan.channel(36).bandwidthHz());
        assertThrows(IllegalArgumentException.class, () -> ApRadioPlan.channel(14));
    }

    @Test
    void dataRateClimbsWithRssi() {
        // Noise floor (20 MHz, NF 6) is -95 dBm; 3 dB margin.
        assertEquals("OFDM-54", ApRadioPlan.dataRate(6, -40).modulation());
        assertEquals(54_000, ApRadioPlan.dataRate(6, -40).kbps());
        assertEquals("OFDM-6", ApRadioPlan.dataRate(6, -87).modulation());     // SINR ~5 dB: only 6 Mbit/s
        assertEquals("DSSS-1", ApRadioPlan.dataRate(6, -95).modulation());     // below every OFDM rate
        assertEquals("OFDM-6", ApRadioPlan.dataRate(36, -95).modulation());    // 5 GHz has no DSSS
        assertEquals("DSSS-1", ApRadioPlan.dataRate(6, Double.NaN).modulation());
        int last = 0;
        for (int rssi = -95; rssi <= -40; rssi++) {
            int k = ApRadioPlan.dataRate(6, rssi).kbps();
            assertTrue(k >= last, "rate fell at " + rssi);
            last = k;
        }
    }

    @Test
    void airtimeMatchesThePhy() {
        // DSSS 1 Mbit/s: 192 us preamble + (100 + 4 FCS) * 8 us.
        assertEquals(192 + 104 * 8, ApRadioPlan.airtimeUs(ApRadioPlan.DSSS_1, 100, 6));
        // OFDM 54 on 5 GHz, 1500-byte MPDU: 16+12032+6 = 12054 bits / 216 per symbol = 56 symbols.
        assertEquals(20 + 56 * 4, ApRadioPlan.airtimeUs(new ApRadioPlan.Rate("OFDM-54", 54_000), 1500, 36));
        // Same on 2.4 GHz adds the 6 us signal extension.
        assertEquals(20 + 56 * 4 + 6, ApRadioPlan.airtimeUs(new ApRadioPlan.Rate("OFDM-54", 54_000), 1500, 6));
        // A ~100-byte beacon every 102.4 ms at 1 Mbit/s is about 1% airtime.
        double share = ApRadioPlan.airtimeUs(ApRadioPlan.DSSS_1, 100, 1) / 102_400.0;
        assertTrue(share > 0.005 && share < 0.015, "beacon airtime share " + share);
    }

    @Test
    void antennaIsAnOmniWithNulls() {
        ApAntenna a = ApAntenna.INSTANCE;
        assertEquals(5, a.gainDbi(1, 0, 0), 1e-9);
        assertEquals(5, a.gainDbi(0, 0, -1), 1e-9);
        assertEquals(ApAntenna.NULL_DBI, a.gainDbi(0, 1, 0), 1e-9);
        assertTrue(a.gainDbi(1, 1, 0) < a.gainDbi(1, 0, 0));
        assertEquals(5, a.peakGainDbi());
    }

    // ------------------------------------------------------------ settings

    @Test
    void defaultsAreValidAndOpen() {
        ApSettings d = ApSettings.defaults(BSSID);
        assertEquals("ECM-5678", d.ssid());
        assertEquals(Security.OPEN, d.security());
        assertEquals(0, d.channelSetting());
        assertNull(d.validate(""));
        ApConfig cfg = d.toConfig(BSSID, 6, "");
        assertEquals(Security.OPEN, cfg.security());
        assertEquals(6, cfg.channel());
    }

    @Test
    void validationCatchesBadSettings() {
        ApSettings ok = new ApSettings("Home", false, Security.WPA2_PSK, 6, 17, true, ApConfig.MacFilterMode.DENY,
                List.of(MacAddress.parse("02:00:00:00:00:01")));
        assertNull(ok.validate("correct horse"));
        assertEquals("WPA2 needs a passphrase", ok.validate(""));
        assertNotNull(ok.validate("short"));
        assertNotNull(withSsid("").validate(null));
        assertNotNull(withSsid("x".repeat(33)).validate(null));
        assertNotNull(new ApSettings("a", false, Security.OPEN, 7, 10, false, null, null).validate(null));     // not offered
        assertNotNull(new ApSettings("a", false, Security.OPEN, 0, 21, false, null, null).validate(null));     // > 20 dBm
        assertNotNull(new ApSettings("a", false, Security.OPEN, 0, 10, false, null,
                List.of(MacAddress.BROADCAST)).validate(null));
        ApConfig cfg = ok.toConfig(BSSID, 6, "correct horse");
        assertTrue(cfg.clientIsolation());
        assertFalse(cfg.macAllowed(MacAddress.parse("02:00:00:00:00:01")));
        assertTrue(cfg.macAllowed(MacAddress.parse("02:00:00:00:00:02")));
    }

    private static ApSettings withSsid(String s) {
        return new ApSettings(s, false, Security.OPEN, 0, 20, false, null, null);
    }

    @Test
    void macListsParseAndFormat() {
        List<MacAddress> l = ApSettings.parseMacList("02:00:00:00:00:01, 02-00-00-00-00-02\n02:00:00:00:00:01");
        assertEquals(2, l.size());
        assertEquals("02:00:00:00:00:01, 02:00:00:00:00:02", ApSettings.formatMacList(l));
        assertTrue(ApSettings.parseMacList("  ").isEmpty());
        IllegalArgumentException e = assertThrows(IllegalArgumentException.class, () -> ApSettings.parseMacList("02:00:zz"));
        assertTrue(e.getMessage().contains("02:00:zz"));
    }

    @Test
    void autoChannelPicksTheQuietest() {
        assertEquals(6, ApSettings.pickAutoChannel(Map.of()));
        assertEquals(1, ApSettings.pickAutoChannel(Map.of(6, -50.0, 11, -60.0)));
        assertEquals(11, ApSettings.pickAutoChannel(Map.of(6, -50.0, 1, -55.0, 11, -80.0)));
    }

    // ------------------------------------------------------------ IPv4 responder

    static final MacAddress ME = MacAddress.parse("02:5a:00:00:00:09");
    static final MacAddress PEER = MacAddress.parse("02:00:11:22:33:44");

    @Test
    void answersArpForItsAddressOnly() {
        int me = IpResponder.ip("10.0.5.20");
        byte[] req = arpRequest(IpResponder.ip("10.0.5.1"), me);
        byte[] rep = IpResponder.respond(req, ME, me);
        assertNotNull(rep);
        assertEquals(PEER, MacAddress.read(rep, 0));
        assertEquals(ME, MacAddress.read(rep, 6));
        assertEquals(2, rep[21]);                                  // op = reply
        assertEquals(ME, MacAddress.read(rep, 22));                // sender hardware
        assertEquals(PEER, MacAddress.read(rep, 32));              // target hardware
        assertNull(IpResponder.respond(arpRequest(IpResponder.ip("10.0.5.1"), IpResponder.ip("10.0.5.21")), ME, me));
    }

    @Test
    void answersPingWithValidChecksums() {
        int me = IpResponder.ip("10.0.5.20"), peer = IpResponder.ip("10.0.5.1");
        byte[] req = echoRequest(peer, me, new byte[] {1, 2, 3, 4, 5});
        byte[] rep = IpResponder.respond(req, ME, me);
        assertNotNull(rep);
        assertEquals(PEER, MacAddress.read(rep, 0));
        assertEquals(0, rep[34]);                                  // echo reply
        assertEquals(0, IpResponder.checksum(rep, 14, 20), "IPv4 header checksum");
        assertEquals(0, IpResponder.checksum(rep, 34, rep.length - 34), "ICMP checksum");
        assertEquals(me, readInt(rep, 26));
        assertEquals(peer, readInt(rep, 30));
        assertEquals(5, rep[rep.length - 1]);
        assertNull(IpResponder.respond(echoRequest(peer, me + 1, new byte[4]), ME, me), "not for us");
    }

    private static byte[] arpRequest(int spa, int tpa) {
        byte[] f = new byte[42];
        MacAddress.BROADCAST.write(f, 0);
        PEER.write(f, 6);
        f[12] = 0x08; f[13] = 0x06;
        f[15] = 1; f[16] = 0x08; f[18] = 6; f[19] = 4; f[21] = 1;
        PEER.write(f, 22);
        writeInt(f, 28, spa);
        writeInt(f, 38, tpa);
        return f;
    }

    private static byte[] echoRequest(int src, int dst, byte[] data) {
        int total = 20 + 8 + data.length;
        byte[] f = new byte[14 + total];
        ME.write(f, 0);
        PEER.write(f, 6);
        f[12] = 0x08;
        f[14] = 0x45;
        f[16] = (byte) (total >> 8); f[17] = (byte) total;
        f[22] = 64; f[23] = 1;
        writeInt(f, 26, src);
        writeInt(f, 30, dst);
        int c = IpResponder.checksum(f, 14, 20);
        f[24] = (byte) (c >> 8); f[25] = (byte) c;
        f[34] = 8;
        f[38] = 0x12; f[39] = 0x34; f[41] = 1;
        System.arraycopy(data, 0, f, 42, data.length);
        int ic = IpResponder.checksum(f, 34, 8 + data.length);
        f[36] = (byte) (ic >> 8); f[37] = (byte) ic;
        return f;
    }

    private static void writeInt(byte[] b, int o, int v) {
        b[o] = (byte) (v >>> 24); b[o + 1] = (byte) (v >>> 16); b[o + 2] = (byte) (v >>> 8); b[o + 3] = (byte) v;
    }

    private static int readInt(byte[] b, int o) {
        return ((b[o] & 0xFF) << 24) | ((b[o + 1] & 0xFF) << 16) | ((b[o + 2] & 0xFF) << 8) | (b[o + 3] & 0xFF);
    }
}
