package com.example.evanscomputermod.radio.wifi80211;

import com.example.evanscomputermod.radio.wifi80211.ap.AccessPointCore;
import com.example.evanscomputermod.radio.wifi80211.ap.ApConfig;
import com.example.evanscomputermod.radio.wifi80211.ap.ApOutput;
import com.example.evanscomputermod.radio.wifi80211.crypto.Ccmp;
import com.example.evanscomputermod.radio.wifi80211.frame.EapolKey;
import com.example.evanscomputermod.radio.wifi80211.frame.EthernetFrame;
import com.example.evanscomputermod.radio.wifi80211.frame.Fcs;
import com.example.evanscomputermod.radio.wifi80211.frame.Frame80211;
import com.example.evanscomputermod.radio.wifi80211.frame.FrameControl;
import com.example.evanscomputermod.radio.wifi80211.frame.Frames;
import com.example.evanscomputermod.radio.wifi80211.frame.Ie;
import com.example.evanscomputermod.radio.wifi80211.frame.Kde;
import com.example.evanscomputermod.radio.wifi80211.frame.Llc;
import com.example.evanscomputermod.radio.wifi80211.frame.MacHeader;
import com.example.evanscomputermod.radio.wifi80211.frame.Mgmt;
import com.example.evanscomputermod.radio.wifi80211.frame.PcapWriter;
import com.example.evanscomputermod.radio.wifi80211.frame.Radiotap;
import com.example.evanscomputermod.radio.wifi80211.frame.RsnIe;
import com.example.evanscomputermod.radio.wifi80211.sta.StaOutput;
import com.example.evanscomputermod.radio.wifi80211.sta.StationCore;
import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.List;
import java.util.Random;

import static org.junit.jupiter.api.Assertions.*;

/** Frame encode/decode round trips and byte-exact golden frames (src/test/resources/radio/frames). */
class FrameCodecTest {

    static final MacAddress AP = MacAddress.parse("02:00:00:00:00:01");
    static final MacAddress STA = MacAddress.parse("02:00:00:00:00:02");

    @Test
    void macAddressBasics() {
        assertEquals("02:00:00:00:00:01", AP.toString());
        assertEquals(AP, MacAddress.of(AP.bytes()));
        assertTrue(MacAddress.BROADCAST.isGroup());
        assertTrue(MacAddress.parse("01:00:5e:00:00:01").isGroup());
        assertFalse(STA.isGroup());
        assertTrue(AP.compareTo(STA) < 0);
        assertTrue(MacAddress.parse("80:00:00:00:00:00").compareTo(AP) > 0); // unsigned octet order
    }

    @Test
    void headerVariantsRoundTrip() {
        MacHeader[] hs = {
                MacHeader.mgmt(FrameControl.BEACON, MacAddress.BROADCAST, AP, AP, 4095),
                MacHeader.data(FrameControl.TO_DS, AP, STA, MacAddress.BROADCAST, 17),
                MacHeader.qosData(FrameControl.FROM_DS | FrameControl.RETRY, STA, AP, AP, 33, 6),
                new MacHeader(FrameControl.of(FrameControl.TYPE_DATA, FrameControl.QOS_DATA,
                        FrameControl.TO_DS | FrameControl.FROM_DS | FrameControl.ORDER), 44, AP, STA, AP, 0x1231, STA, 0x0025, 0x12345678),
                new MacHeader(FrameControl.of(FrameControl.TYPE_CTRL, FrameControl.ACK), 0, STA, null, null, -1, null, -1, -1),
                new MacHeader(FrameControl.of(FrameControl.TYPE_CTRL, FrameControl.RTS), 300, AP, STA, null, -1, null, -1, -1),
        };
        int[] lengths = {24, 24, 26, 36, 10, 16};
        for (int i = 0; i < hs.length; i++) {
            byte[] enc = hs[i].encode();
            assertEquals(lengths[i], enc.length, "header " + i);
            assertEquals(hs[i], MacHeader.parse(enc), "header " + i);
        }
        assertEquals(4095, hs[0].sequenceNumber());
        assertEquals(6, hs[2].tid());
        assertThrows(IllegalArgumentException.class, () -> MacHeader.parse(new byte[12]));
        assertThrows(IllegalArgumentException.class, () -> MacHeader.parse(new byte[]{0x0c, 0, 0, 0, 0, 0, 0, 0, 0, 0})); // type 3
    }

    @Test
    void managementBodiesRoundTrip() {
        Mgmt.Beacon b = new Mgmt.Beacon(0x0102030405060708L, 100, 0x0411, List.of(Ie.ssid("ECM".getBytes()), Ie.dsParams(11)));
        Mgmt.Beacon b2 = Mgmt.Beacon.parse(b.encode());
        assertEquals(b.timestamp(), b2.timestamp());
        assertEquals(b.ies(), b2.ies());
        assertArrayEquals("ECM".getBytes(), b2.ssid());

        Mgmt.Auth a = new Mgmt.Auth(0, 2, 13, List.of());
        assertEquals(a, Mgmt.Auth.parse(a.encode()));

        Mgmt.AssocRequest re = new Mgmt.AssocRequest(0x0401, 10, AP, List.of(RsnIe.wpa2PskCcmp().toIe()));
        assertEquals(re, Mgmt.AssocRequest.parse(re.encode(), true));
        Mgmt.AssocResponse r = new Mgmt.AssocResponse(0x0411, 0, 2007, List.of());
        assertEquals(0xC7, r.encode()[5] & 0xFF); // AID 2007 = 0x07D7 | 0xC000
        assertEquals(r, Mgmt.AssocResponse.parse(r.encode()));
        assertEquals(15, Mgmt.parseReason(Mgmt.reason(15)));

        assertThrows(IllegalArgumentException.class, () -> Ie.parseAll(new byte[]{0, 5, 1}, 0, 3));
    }

    @Test
    void rsnElement() {
        RsnIe r = RsnIe.wpa2PskCcmp();
        assertEquals("30140100000fac040100000fac040100000fac020000", Hex.of(r.encode()));
        assertEquals(r, RsnIe.parse(r.toIe().data()));
        assertTrue(r.isWpa2PskCcmp());
        // Optional fields: version only means CCMP group/pairwise and 802.1X AKM defaults.
        RsnIe minimal = RsnIe.parse(new byte[]{1, 0});
        assertEquals(List.of(RsnIe.AKM_8021X), minimal.akms());
        assertFalse(minimal.isWpa2PskCcmp());
    }

    @Test
    void llcSnapAndEthernet() {
        byte[] p = {1, 2, 3};
        Llc.Decap d = Llc.decap(Llc.encap(0x0800, p));
        assertEquals(0x0800, d.etherType());
        assertArrayEquals(p, d.payload());
        assertEquals("aaaa03000000080001", Hex.of(Llc.encap(0x0800, new byte[]{1})).substring(0, 18));
        assertEquals("aaaa030000f8", Hex.of(Llc.encap(0x8137, p)).substring(0, 12)); // IPX: bridge tunnel
        // 802.3 (length) frames carry their own LLC header untouched.
        byte[] stp = {0x42, 0x42, 0x03, 0, 0};
        assertArrayEquals(stp, Llc.encap(stp.length, stp));
        assertEquals(stp.length, Llc.decap(stp).etherType());

        EthernetFrame e = new EthernetFrame(MacAddress.BROADCAST, STA, 0x0806, new byte[]{9, 9});
        assertEquals(e, EthernetFrame.parse(e.encode()));
    }

    @Test
    void eapolKeyCodecAndKde() {
        byte[] nonce = new byte[32];
        nonce[0] = 7;
        EapolKey k = EapolKey.of(EapolKey.KI_M3, 16, 0x0102030405060708L, nonce, EapolKey.rscFromPn(0x010203040506L), new byte[24]);
        byte[] enc = k.encode();
        assertEquals(99 + 24, enc.length);
        assertEquals(0x13CA, (enc[5] & 0xFF) << 8 | (enc[6] & 0xFF));
        assertEquals(k, EapolKey.parse(enc));
        assertEquals(0x010203040506L, EapolKey.pnFromRsc(EapolKey.parse(enc).rsc()));
        assertEquals(0x008A, EapolKey.KI_M1);
        assertEquals(0x010A, EapolKey.KI_M2);
        assertEquals(0x030A, EapolKey.KI_M4);
        assertEquals(0x1382, EapolKey.KI_G1);
        assertEquals(0x0302, EapolKey.KI_G2);

        byte[] gtk = new byte[16];
        gtk[15] = 1;
        byte[] kde = Kde.gtk(2, false, gtk);
        assertEquals("dd16000fac010200", Hex.of(kde).substring(0, 16));
        byte[] padded = Kde.pad(Kde.concat(RsnIe.wpa2PskCcmp().encode(), kde));
        assertEquals(0, padded.length % 8);
        assertEquals(22 + 24 + 2, padded.length); // 46 -> 48 with DD 00
        assertEquals(0xDD, padded[46] & 0xFF);
        Kde.Gtk g = Kde.findGtk(padded);
        assertEquals(2, g.keyId());
        assertArrayEquals(gtk, g.key());
        assertArrayEquals(RsnIe.wpa2PskCcmp().encode(), Kde.findElement(padded, Ie.RSN));
        assertEquals(16, Kde.pad(new byte[3]).length);
    }

    @Test
    void fcsRadiotapAndPcap() {
        byte[] ack = Frames.ack(STA);
        byte[] withFcs = Fcs.append(ack);
        assertTrue(Fcs.verify(withFcs));
        withFcs[2] ^= 1;
        assertNull(Fcs.strip(withFcs));

        byte[] rt = Radiotap.wrap(ack, 54_000, 6, -40, false);
        assertArrayEquals(Hex.resource("radiotap_ack.hex"), rt);
        Radiotap.Info info = Radiotap.parse(rt);
        assertEquals(15, info.headerLength());
        assertEquals(54_000, info.rateKbps());
        assertEquals(2437, info.frequencyMhz());
        assertEquals(-40, info.signalDbm());
        assertFalse(info.hasFcs());
        assertArrayEquals(ack, Radiotap.payload(rt));
        assertEquals(Radiotap.CHAN_5GHZ | Radiotap.CHAN_OFDM, Radiotap.parse(Radiotap.wrap(ack, 6000, 36, -70, true)).channelFlags());

        byte[] pcap = new PcapWriter(PcapWriter.LINKTYPE_IEEE802_11_RADIO).add(1_500_000, rt).toByteArray();
        assertEquals(24 + 16 + rt.length, pcap.length);
        assertEquals("d4c3b2a1", Hex.of(pcap).substring(0, 8));
        assertEquals(127, pcap[20]);
        assertEquals(1, pcap[24]);
        assertEquals(0x20, pcap[28] & 0xFF); // 500000 us = 0x07A120
    }

    @Test
    void channels() {
        assertEquals(2412, Channels.frequencyMhz(1));
        assertEquals(2484, Channels.frequencyMhz(14));
        assertEquals(5180, Channels.frequencyMhz(36));
        assertEquals(11, Channels.channelFor(2462));
        assertEquals(149, Channels.channelFor(5745));
    }

    // ------------------------------------------------------------------ golden frames

    @Test
    void goldenBeaconsFromAccessPoint() {
        List<byte[]> air = new ArrayList<>();
        ApOutput out = new ApOutput() {
            @Override public void transmitRadio(byte[] f) { air.add(f); }
            @Override public void transmitWired(byte[] e, MacAddress src) { }
        };
        new AccessPointCore(ApConfig.builder(AP, "ECM").wpa2("password1").channel(6).build(), out, new Random(1)).tick(100);
        assertEquals(Hex.of(Hex.resource("beacon_wpa2_ch6.hex")), Hex.of(air.get(0)));
        air.clear();
        new AccessPointCore(ApConfig.builder(AP, "ECM").open().hidden(true).channel(1).build(), out, new Random(1)).tick(0);
        assertEquals(Hex.of(Hex.resource("beacon_hidden_open_ch1.hex")), Hex.of(air.get(0)));
    }

    @Test
    void goldenStationFrames() {
        List<byte[]> air = new ArrayList<>();
        StationCore sta = new StationCore(STA, new StaOutput() {
            @Override public void transmitRadio(byte[] f) { air.add(f); }
            @Override public void deliverEthernet(byte[] e) { }
        }, new Random(1));
        sta.probe("ECM", 6, 0);
        assertEquals(Hex.of(Hex.resource("probe_req_directed.hex")), Hex.of(air.get(0)));
    }

    @Test
    void goldenHandBuiltFrames() {
        byte[] auth = Frames.mgmt(FrameControl.AUTH, AP, STA, AP, 1, new Mgmt.Auth(0, 1, 0, List.of()).encode());
        assertGolden("auth_req_open.hex", auth);

        List<Ie> ies = new ArrayList<>();
        ies.add(Ie.ssid("ECM".getBytes()));
        ies.add(Ie.rates(6).get(0));
        ies.add(RsnIe.wpa2PskCcmp().toIe());
        ies.add(Ie.rates(6).get(1));
        assertGolden("assoc_req_wpa2.hex", Frames.mgmt(FrameControl.ASSOC_REQ, AP, STA, AP, 3,
                new Mgmt.AssocRequest(0x0401, 10, null, ies).encode()));
        assertGolden("assoc_resp_aid1.hex", Frames.mgmt(FrameControl.ASSOC_RESP, STA, AP, AP, 2,
                new Mgmt.AssocResponse(0x0411, 0, 1, Ie.rates(6)).encode()));
        assertGolden("deauth_reason15.hex", Frames.deauth(STA, AP, AP, 5, Mgmt.REASON_4WAY_TIMEOUT));

        byte[] anonce = new byte[32];
        for (int i = 0; i < 32; i++) anonce[i] = (byte) i;
        byte[] m1 = EapolKey.of(EapolKey.KI_M1, 16, 1, anonce, null, null).encode();
        assertGolden("eapol_m1.hex", Frames.data(FrameControl.FROM_DS, STA, AP, AP, 4, EthernetFrame.ETHERTYPE_EAPOL, m1));
        assertGolden("data_uplink_open.hex", Frames.data(FrameControl.TO_DS, AP, STA, MacAddress.BROADCAST, 7, 0x0806,
                Hex.bytes("deadbeef")));

        // Golden frames parse back to what they encode.
        for (String name : List.of("auth_req_open.hex", "assoc_req_wpa2.hex", "assoc_resp_aid1.hex", "deauth_reason15.hex",
                "eapol_m1.hex", "data_uplink_open.hex", "beacon_wpa2_ch6.hex", "probe_req_directed.hex")) {
            byte[] g = Hex.resource(name);
            assertArrayEquals(g, Frame80211.parse(g).encode(), name);
        }
        Frame80211 m1f = Frame80211.parse(Hex.resource("eapol_m1.hex"));
        EapolKey parsed = EapolKey.parse(Llc.decap(m1f.body()).payload());
        assertEquals(EapolKey.KI_M1, parsed.keyInfo());
        assertArrayEquals(anonce, parsed.nonce());
    }

    @Test
    void goldenCcmpMpduWithFcs() {
        byte[] g = Hex.resource("ccmp_annex_j64_fcs.hex");
        byte[] mpdu = Fcs.strip(g);
        assertNotNull(mpdu, "FCS must verify");
        Ccmp.Decrypted d = Ccmp.decrypt(Hex.bytes(Wpa2CryptoVectorsTest.CCMP_TK), mpdu);
        assertNotNull(d);
        assertEquals("f8ba1a55d02f85ae967bb62fb6cda8eb7e78a050", Hex.of(Frame80211.parse(d.mpdu()).body()));
    }

    private static void assertGolden(String name, byte[] actual) {
        assertEquals(Hex.of(Hex.resource(name)), Hex.of(actual), name);
    }
}
