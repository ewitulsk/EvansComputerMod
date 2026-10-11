package com.example.evanscomputermod.radio.wifi80211;

import com.example.evanscomputermod.radio.wifi80211.ap.ApConfig;
import com.example.evanscomputermod.radio.wifi80211.ap.ClientStatus;
import com.example.evanscomputermod.radio.wifi80211.crypto.Ccmp;
import com.example.evanscomputermod.radio.wifi80211.frame.EapolKey;
import com.example.evanscomputermod.radio.wifi80211.frame.EthernetFrame;
import com.example.evanscomputermod.radio.wifi80211.frame.Frame80211;
import com.example.evanscomputermod.radio.wifi80211.frame.FrameControl;
import com.example.evanscomputermod.radio.wifi80211.frame.Frames;
import com.example.evanscomputermod.radio.wifi80211.frame.Ie;
import com.example.evanscomputermod.radio.wifi80211.frame.Llc;
import com.example.evanscomputermod.radio.wifi80211.frame.Mgmt;
import com.example.evanscomputermod.radio.wifi80211.sta.ScanResult;
import com.example.evanscomputermod.radio.wifi80211.sta.StationCore;
import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.Set;

import static org.junit.jupiter.api.Assertions.*;

/**
 * End-to-end AP state machine against the Java station on an ideal medium:
 * association, the 4-way handshake, CCMP data both ways, bridging and the
 * required negative cases (wrong passphrase, replay, lost M3, deauth,
 * inactivity, hidden SSID, GTK rekey).
 */
class AccessPointCoreTest {

    static final MacAddress BSSID = MacAddress.parse("02:00:00:00:00:01");
    static final MacAddress STA1 = MacAddress.parse("02:00:00:00:00:02");
    static final MacAddress STA2 = MacAddress.parse("02:00:00:00:00:03");
    static final MacAddress WIRED_HOST = MacAddress.parse("02:aa:00:00:00:10");
    static final String SSID = "ECM-Lab";
    static final String PASS = "correct horse battery";

    private static ApConfig.Builder wpa2() {
        return ApConfig.builder(BSSID, SSID).wpa2(PASS).channel(6);
    }

    private static byte[] payload(String s) {
        return s.getBytes(StandardCharsets.US_ASCII);
    }

    private static boolean containsBytes(byte[] hay, byte[] needle) {
        outer:
        for (int i = 0; i + needle.length <= hay.length; i++) {
            for (int j = 0; j < needle.length; j++) if (hay[i + j] != needle[j]) continue outer;
            return true;
        }
        return false;
    }

    @Test
    void openNetworkAssociatesAndBridges() {
        IdealAir air = new IdealAir(ApConfig.builder(BSSID, SSID).open().channel(11).build(), 1);
        StationCore s = air.addStation(STA1, 2);
        air.advance(110);
        s.probe(null, 11, air.now);
        air.pump();
        ScanResult r = s.scanResults().get(0);
        assertEquals(SSID, r.ssidString());
        assertEquals(11, r.channel());
        assertNull(r.rsn());
        assertFalse(s.connect(SSID, PASS, air.now), "a passphrase for an open network is refused");
        assertTrue(s.connect(SSID, null, air.now));
        air.pump();
        assertEquals(StationCore.State.CONNECTED, s.state());
        assertEquals(1, s.aid());
        ClientStatus c = air.ap.client(STA1);
        assertEquals(ClientStatus.State.AUTHORIZED, c.state());
        assertEquals(-48, c.lastRssiDbm());
        assertEquals(54_000, c.lastRateKbps());
        assertEquals(List.of(STA1), air.authorized);

        air.send(s, WIRED_HOST, payload("hello wire"));
        assertEquals(1, air.wired.size());
        EthernetFrame up = EthernetFrame.parse(air.wired.get(0).eth());
        assertEquals(STA1, air.wired.get(0).src());
        assertEquals(new EthernetFrame(WIRED_HOST, STA1, EthernetFrame.ETHERTYPE_IPV4, payload("hello wire")), up);

        air.wire(STA1, WIRED_HOST, EthernetFrame.ETHERTYPE_IPV4, payload("hello radio"));
        assertEquals(new EthernetFrame(STA1, WIRED_HOST, EthernetFrame.ETHERTYPE_IPV4, payload("hello radio")),
                EthernetFrame.parse(air.delivered.get(STA1).get(0)));
        // Unknown destination on the wire is not ours to send.
        int before = air.log.size();
        air.wire(STA2, WIRED_HOST, EthernetFrame.ETHERTYPE_IPV4, payload("nobody"));
        assertEquals(before, air.log.size());
    }

    @Test
    void wpa2HandshakeThenEncryptedUnicastAndBroadcastBothWays() throws IOException {
        IdealAir air = new IdealAir(wpa2().build(), 7);
        StationCore s1 = air.addStation(STA1, 8);
        StationCore s2 = air.addStation(STA2, 9);
        air.join(s1, SSID, PASS);
        air.join(s2, SSID, PASS);
        for (StationCore s : List.of(s1, s2)) {
            assertEquals(StationCore.State.CONNECTED, s.state(), String.valueOf(s.lastError()));
            assertTrue(s.keysInstalled());
            assertTrue(s.hasGroupKey(1));
            ClientStatus c = air.ap.client(s.mac());
            assertEquals(ClientStatus.State.AUTHORIZED, c.state());
            assertEquals(ClientStatus.Handshake.DONE, c.handshake());
            assertNull(c.lastError());
        }
        assertEquals(1, air.ap.client(STA1).aid());
        assertEquals(2, air.ap.client(STA2).aid());
        assertEquals(2, air.countOnAir(EapolKey.KI_M1));
        assertEquals(2, air.countOnAir(EapolKey.KI_M2));
        assertEquals(2, air.countOnAir(EapolKey.KI_M3));
        assertEquals(2, air.countOnAir(EapolKey.KI_M4));

        // Uplink unicast: encrypted on the air, plaintext Ethernet on the wire with the client's MAC.
        byte[] secret = payload("uplink secret 0123456789");
        int logStart = air.log.size();
        air.send(s1, WIRED_HOST, secret);
        byte[] onAir = air.log.get(logStart).frame();
        assertTrue(IdealAir.isProtectedData(onAir));
        assertFalse(containsBytes(onAir, secret));
        assertEquals(new EthernetFrame(WIRED_HOST, STA1, EthernetFrame.ETHERTYPE_IPV4, secret),
                EthernetFrame.parse(air.wired.get(0).eth()));

        // Downlink unicast under the client's TK.
        byte[] down = payload("downlink secret");
        logStart = air.log.size();
        air.wire(STA2, WIRED_HOST, EthernetFrame.ETHERTYPE_IPV4, down);
        assertEquals(1, air.log.size() - logStart);
        assertFalse(containsBytes(air.log.get(logStart).frame(), down));
        assertEquals(0, Ccmp.peekKeyId(air.log.get(logStart).frame()));
        assertEquals(List.of(), air.delivered.get(STA1));
        assertArrayEquals(down, EthernetFrame.parse(air.delivered.get(STA2).get(0)).payload());

        // Downlink broadcast under the GTK reaches both.
        logStart = air.log.size();
        air.wire(MacAddress.BROADCAST, WIRED_HOST, EthernetFrame.ETHERTYPE_ARP, payload("who-has"));
        assertEquals(1, Ccmp.peekKeyId(air.log.get(logStart).frame()));
        assertEquals(1, air.delivered.get(STA1).size());
        assertEquals(2, air.delivered.get(STA2).size());

        // Uplink broadcast: to the wire and relayed to the other station, not echoed to the sender.
        air.send(s1, MacAddress.BROADCAST, payload("dhcp discover"));
        assertEquals(2, air.wired.size());
        assertEquals(1, air.delivered.get(STA1).size());
        assertEquals(3, air.delivered.get(STA2).size());
        EthernetFrame relayed = EthernetFrame.parse(air.delivered.get(STA2).get(2));
        assertEquals(STA1, relayed.src());
        assertEquals(MacAddress.BROADCAST, relayed.dst());

        // Station to station unicast is relayed by the AP and never touches the wire.
        air.send(s2, STA1, payload("peer to peer"));
        assertEquals(2, air.wired.size());
        EthernetFrame p2p = EthernetFrame.parse(air.delivered.get(STA1).get(1));
        assertEquals(STA2, p2p.src());
        assertArrayEquals(payload("peer to peer"), p2p.payload());

        // A frame from the wire claiming to come from a wireless client is a loop: dropped.
        logStart = air.log.size();
        air.wire(STA2, STA1, EthernetFrame.ETHERTYPE_IPV4, payload("loop"));
        assertEquals(logStart, air.log.size());

        Path dir = Path.of("build", "radio");
        Files.createDirectories(dir);
        Files.write(dir.resolve("wifi80211_wpa2_session.pcap"), air.pcap.toByteArray());
    }

    @Test
    void stationGivesUpAfterBeaconLossAndRejoinsWhenTheApIsBack() {
        IdealAir air = new IdealAir(wpa2().build(), 21);
        StationCore s = air.addStation(STA1, 22);
        air.join(s, SSID, PASS);
        assertEquals(StationCore.State.CONNECTED, s.state(), String.valueOf(s.lastError()));
        // The AP goes out of range: nothing from it reaches the station.
        air.drop = (from, f) -> from == air.ap;
        air.advance(StationCore.BEACON_LOSS_MS - 200);
        assertEquals(StationCore.State.CONNECTED, s.state(), "a few missed beacons are not a lost link");
        air.advance(400);
        assertEquals(StationCore.State.IDLE, s.state());
        assertTrue(s.lastError().startsWith("beacon loss"), s.lastError());
        assertFalse(s.connect(SSID, PASS, air.now), "the lost BSS is forgotten until it is heard again");
        // Back in range: the next beacon brings it back and the station rejoins.
        air.drop = (from, f) -> false;
        air.advance(110);
        assertTrue(s.connect(SSID, PASS, air.now), String.valueOf(s.lastError()));
        air.pump();
        air.advance(5);
        assertEquals(StationCore.State.CONNECTED, s.state(), String.valueOf(s.lastError()));
    }

    @Test
    void clientIsolationBlocksStationToStation() {
        IdealAir air = new IdealAir(wpa2().clientIsolation(true).build(), 3);
        StationCore s1 = air.addStation(STA1, 4);
        StationCore s2 = air.addStation(STA2, 5);
        air.join(s1, SSID, PASS);
        air.join(s2, SSID, PASS);
        air.send(s1, STA2, payload("blocked"));
        assertEquals(List.of(), air.delivered.get(STA2));
        assertEquals(List.of(), air.wired, "isolated peer traffic is dropped, not leaked to the wire");
        air.send(s1, MacAddress.BROADCAST, payload("bcast"));
        assertEquals(1, air.wired.size(), "broadcast still reaches the wired LAN");
        assertEquals(List.of(), air.delivered.get(STA2));
        // The wired side still reaches both.
        air.wire(MacAddress.BROADCAST, WIRED_HOST, EthernetFrame.ETHERTYPE_ARP, payload("arp"));
        assertEquals(1, air.delivered.get(STA1).size());
        assertEquals(1, air.delivered.get(STA2).size());
    }

    @Test
    void wrongPassphraseFailsAtM2MicAndNoKeysAreInstalled() {
        IdealAir air = new IdealAir(wpa2().build(), 11);
        StationCore s = air.addStation(STA1, 12);
        air.join(s, SSID, "not the password");
        ClientStatus c = air.ap.client(STA1);
        assertEquals(ClientStatus.State.ASSOCIATED, c.state());
        assertEquals(ClientStatus.Handshake.PTK_START, c.handshake());
        assertTrue(c.lastError().contains("M2 MIC"), c.lastError());
        assertEquals(1, c.micFailures());
        assertEquals(1, air.countOnAir(EapolKey.KI_M2));
        assertEquals(0, air.countOnAir(EapolKey.KI_M3), "AP must not answer a bad M2");
        assertFalse(s.keysInstalled());
        assertEquals(StationCore.State.HANDSHAKE, s.state());
        assertEquals(List.of(), air.authorized);
        // Data cannot pass.
        assertFalse(s.sendEthernet(new EthernetFrame(WIRED_HOST, STA1, 0x0800, new byte[4]).encode(), air.now));
        air.wire(STA1, WIRED_HOST, 0x0800, payload("x"));
        assertEquals(List.of(), air.delivered.get(STA1));

        // M1 is retried (4 transmissions, 1 s apart), every M2 fails, then the AP deauths with reason 15.
        air.advance(4_000);
        assertEquals(4, air.countOnAir(EapolKey.KI_M1));
        assertEquals(0, air.countOnAir(EapolKey.KI_M3));
        assertNull(air.ap.client(STA1));
        assertEquals(StationCore.State.IDLE, s.state());
        assertEquals(Mgmt.REASON_4WAY_TIMEOUT, s.lastDeauthReason());
        assertTrue(air.ap.recentEvents().stream().anyMatch(e -> e.contains("M2 MIC mismatch")));
        assertEquals(List.of(), air.wired);
    }

    @Test
    void replayedPacketNumbersAreDropped() {
        IdealAir air = new IdealAir(wpa2().build(), 21);
        StationCore s = air.addStation(STA1, 22);
        air.join(s, SSID, PASS);
        int at = air.log.size();
        air.send(s, WIRED_HOST, payload("once"));
        byte[] uplink = air.log.get(at).frame();
        assertEquals(1, air.wired.size());
        air.ap.onReceive(uplink, RxMeta.NONE, air.now);
        assertEquals(1, air.wired.size(), "replayed uplink must not be bridged twice");
        assertEquals(1, air.ap.client(STA1).replayDrops());
        // A later frame with a fresh PN still passes.
        air.send(s, WIRED_HOST, payload("twice"));
        assertEquals(2, air.wired.size());

        at = air.log.size();
        air.wire(STA1, WIRED_HOST, 0x0800, payload("down"));
        byte[] downlink = air.log.get(at).frame();
        s.onReceive(downlink, RxMeta.NONE, air.now);
        assertEquals(1, air.delivered.get(STA1).size());
        assertEquals(1, s.replayDrops());

        // Retry bit set on a retransmission does not let a replay through either.
        byte[] retry = uplink.clone();
        retry[1] |= 0x08;
        air.ap.onReceive(retry, RxMeta.NONE, air.now);
        assertEquals(2, air.wired.size());
        assertEquals(2, air.ap.client(STA1).replayDrops());
    }

    @Test
    void lostM3IsRetransmittedAndLostM4DoesNotReinstallKeys() {
        IdealAir air = new IdealAir(wpa2().build(), 31);
        StationCore s = air.addStation(STA1, 32);
        int[] m3Seen = {0};
        air.drop = (from, f) -> IdealAir.eapolKeyInfo(f) == EapolKey.KI_M3 && m3Seen[0]++ == 0;
        air.join(s, SSID, PASS);
        assertEquals(StationCore.State.HANDSHAKE, s.state());
        assertEquals(ClientStatus.Handshake.PTK_NEGOTIATING, air.ap.client(STA1).handshake());
        air.advance(1_000);
        assertEquals(2, air.countOnAir(EapolKey.KI_M3));
        assertEquals(StationCore.State.CONNECTED, s.state());
        assertEquals(ClientStatus.State.AUTHORIZED, air.ap.client(STA1).state());
        // The retransmitted M3 used a higher replay counter.
        List<Long> counters = air.log.stream().filter(t -> IdealAir.eapolKeyInfo(t.frame()) == EapolKey.KI_M3)
                .map(t -> EapolKey.parse(Llc.decap(
                        Frame80211.parse(t.frame()).body()).payload()).replayCounter()).toList();
        assertTrue(counters.get(1) > counters.get(0));
        air.send(s, WIRED_HOST, payload("after lost M3"));
        assertEquals(1, air.wired.size());

        // Second station: its first M4 is lost; the AP repeats M3, the station answers again
        // (encrypted now) without reinstalling the TK, and data still flows.
        StationCore s2 = air.addStation(STA2, 33);
        int[] m4Seen = {0};
        air.drop = (from, f) -> from == s2 && IdealAir.eapolKeyInfo(f) == EapolKey.KI_M4 && m4Seen[0]++ == 0;
        air.join(s2, SSID, PASS);
        assertEquals(StationCore.State.CONNECTED, s2.state());
        assertEquals(ClientStatus.Handshake.PTK_NEGOTIATING, air.ap.client(STA2).handshake());
        air.send(s2, WIRED_HOST, payload("pn 1")); // AP has no key yet: dropped
        assertEquals(1, air.wired.size());
        air.advance(1_000);
        assertEquals(2, s2.m4Sent());
        assertEquals(ClientStatus.State.AUTHORIZED, air.ap.client(STA2).state());
        air.send(s2, WIRED_HOST, payload("pn continues"));
        assertEquals(2, air.wired.size());
        assertArrayEquals(payload("pn continues"), EthernetFrame.parse(air.wired.get(1).eth()).payload());
    }

    /**
     * The Rust ecm-wifi station sends every EAPOL-Key frame unencrypted and may use
     * QoS Data uplink. The AP must accept plaintext M4 (also when repeated after a
     * lost M4) and plaintext group message 2, and bridge/replay-check QoS frames.
     */
    @Test
    void plaintextEapolAndQosUplinkLikeTheRustStation() {
        IdealAir air = new IdealAir(wpa2().gtkRekeyIntervalMs(5_000).build(), 111);
        StationCore s = air.addStation(STA1, 112);
        s.setPlaintextEapol(true);
        s.setQosData(true);
        int[] m4Seen = {0};
        air.drop = (from, f) -> IdealAir.eapolKeyInfo(f) == EapolKey.KI_M4 && m4Seen[0]++ == 0;
        air.join(s, SSID, PASS);
        air.advance(1_000);
        assertEquals(2, air.countOnAir(EapolKey.KI_M4), "both M4s travel in the clear");
        assertEquals(ClientStatus.State.AUTHORIZED, air.ap.client(STA1).state());

        int at = air.log.size();
        air.send(s, WIRED_HOST, payload("qos uplink"));
        Frame80211 qos = Frame80211.parse(air.log.get(at).frame());
        assertEquals(FrameControl.QOS_DATA, qos.header().subtype());
        assertTrue(qos.header().isProtected());
        assertEquals(1, air.wired.size());
        assertArrayEquals(payload("qos uplink"), EthernetFrame.parse(air.wired.get(0).eth()).payload());
        air.ap.onReceive(air.log.get(at).frame(), RxMeta.NONE, air.now);
        assertEquals(1, air.wired.size());
        assertEquals(1, air.ap.client(STA1).replayDrops());

        air.advance(5_000);
        assertEquals(1, air.countOnAir(EapolKey.KI_G2), "group message 2 in the clear");
        assertEquals(2, air.ap.currentGtkKeyId());
        assertFalse(air.ap.rekeyInProgress());
        assertNotNull(air.ap.client(STA1));
    }

    @Test
    void deauthenticationBothWays() {
        IdealAir air = new IdealAir(wpa2().build(), 41);
        StationCore s1 = air.addStation(STA1, 42);
        StationCore s2 = air.addStation(STA2, 43);
        air.join(s1, SSID, PASS);
        air.join(s2, SSID, PASS);

        s1.disconnect(Mgmt.REASON_DEAUTH_LEAVING, air.now);
        air.pump();
        assertNull(air.ap.client(STA1));
        assertEquals(List.of(STA1), air.removed);
        assertEquals(StationCore.State.IDLE, s1.state());
        // Its AID is free again for the next station.
        air.join(s1, SSID, PASS);
        assertEquals(1, air.ap.client(STA1).aid());

        assertTrue(air.ap.deauthenticate(STA2, Mgmt.REASON_UNSPECIFIED));
        air.pump();
        assertEquals(StationCore.State.IDLE, s2.state());
        assertEquals(Mgmt.REASON_UNSPECIFIED, s2.lastDeauthReason());
        assertNull(air.ap.client(STA2));

        // Data from a station the AP no longer knows earns a class-3 deauth.
        int at = air.log.size();
        air.ap.onReceive(Frames.data(FrameControl.TO_DS, BSSID, STA2, WIRED_HOST, 9, 0x0800, new byte[4]), RxMeta.NONE, air.now);
        air.pump();
        Frame80211 reply = Frame80211.parse(air.log.get(at).frame());
        assertEquals(FrameControl.DEAUTH, reply.header().subtype());
        assertEquals(Mgmt.REASON_CLASS3_FROM_NONASSOC, Mgmt.parseReason(reply.body()));

        // Disassociation keeps the station authenticated but stops data.
        int wiredBefore = air.wired.size();
        air.ap.onReceive(Frames.disassoc(BSSID, STA1, BSSID, 10, Mgmt.REASON_DISASSOC_LEAVING), RxMeta.NONE, air.now);
        assertEquals(ClientStatus.State.AUTHENTICATED, air.ap.client(STA1).state());
        assertEquals(0, air.ap.client(STA1).aid());
        s1.sendEthernet(new EthernetFrame(WIRED_HOST, STA1, 0x0800, new byte[4]).encode(), air.now);
        air.pump();
        assertEquals(wiredBefore, air.wired.size());
    }

    @Test
    void inactivityTimeoutDeauthenticates() {
        IdealAir air = new IdealAir(wpa2().inactivityTimeoutMs(2_000).build(), 51);
        StationCore s = air.addStation(STA1, 52);
        air.join(s, SSID, PASS);
        air.advance(1_500);
        air.send(s, WIRED_HOST, payload("still here"));
        air.advance(1_500);
        assertNotNull(air.ap.client(STA1), "activity resets the timer");
        air.advance(600);
        assertNull(air.ap.client(STA1));
        assertEquals(StationCore.State.IDLE, s.state());
        assertEquals(Mgmt.REASON_INACTIVITY, s.lastDeauthReason());
        assertEquals(List.of(STA1), air.removed);
    }

    @Test
    void hiddenSsidAnswersOnlyDirectedProbes() {
        IdealAir air = new IdealAir(wpa2().hidden(true).build(), 61);
        StationCore s = air.addStation(STA1, 62);
        air.advance(110);
        ScanResult seen = s.scanResults().get(0);
        assertTrue(seen.hidden(), "beacons carry an empty SSID");
        assertNotNull(seen.rsn());

        int at = air.log.size();
        s.probe(null, 6, air.now);
        air.pump();
        assertEquals(1, air.log.size() - at, "wildcard probe gets no response");
        assertFalse(s.connect(SSID, PASS, air.now));

        s.probe("Some-Other-Net", 6, air.now);
        air.pump();
        assertEquals(2, air.log.size() - at, "probe for another SSID gets no response");

        s.probe(SSID, 6, air.now);
        air.pump();
        assertEquals(4, air.log.size() - at);
        assertEquals(FrameControl.PROBE_RESP, Frame80211.parse(air.log.get(air.log.size() - 1).frame()).header().subtype());
        assertEquals(SSID, s.scanResults().get(0).ssidString());
        air.advance(200); // later hidden beacons must not erase what the probe revealed
        assertTrue(s.connect(SSID, PASS, air.now));
        air.pump();
        assertEquals(StationCore.State.CONNECTED, s.state());
    }

    @Test
    void gtkRekeyRunsGroupHandshakeAndSwitchesKeys() {
        IdealAir air = new IdealAir(wpa2().gtkRekeyIntervalMs(5_000).build(), 71);
        StationCore s1 = air.addStation(STA1, 72);
        StationCore s2 = air.addStation(STA2, 73);
        air.join(s1, SSID, PASS);
        air.join(s2, SSID, PASS);
        assertEquals(1, air.ap.currentGtkKeyId());
        air.advance(5_000);
        assertFalse(air.ap.rekeyInProgress());
        assertEquals(2, air.ap.currentGtkKeyId());
        assertTrue(s1.hasGroupKey(2));
        assertTrue(s2.hasGroupKey(2));
        assertTrue(air.ap.recentEvents().stream().anyMatch(e -> e.contains("GTK rekey complete (key 2)")));

        int at = air.log.size();
        air.wire(MacAddress.BROADCAST, WIRED_HOST, EthernetFrame.ETHERTYPE_ARP, payload("after rekey"));
        assertEquals(2, Ccmp.peekKeyId(air.log.get(at).frame()));
        assertArrayEquals(payload("after rekey"), EthernetFrame.parse(air.delivered.get(STA1).get(0)).payload());
        assertArrayEquals(payload("after rekey"), EthernetFrame.parse(air.delivered.get(STA2).get(0)).payload());

        // A station that never answers group message 1 is deauthenticated with reason 16;
        // the rekey then completes for the rest.
        air.drop = (from, f) -> from == s2 && IdealAir.isProtectedData(f);
        air.advance(5_000 + 4_000);
        assertNull(air.ap.client(STA2));
        assertEquals(Mgmt.REASON_GROUP_KEY_TIMEOUT, s2.lastDeauthReason());
        assertEquals(1, air.ap.currentGtkKeyId());
        assertFalse(air.ap.rekeyInProgress());
        air.drop = (from, f) -> false;
        air.wire(MacAddress.BROADCAST, WIRED_HOST, EthernetFrame.ETHERTYPE_ARP, payload("key 1 again"));
        assertArrayEquals(payload("key 1 again"), EthernetFrame.parse(air.delivered.get(STA1).get(1)).payload());
    }

    @Test
    void macFilterAndRsnPolicy() {
        IdealAir air = new IdealAir(wpa2().macFilter(ApConfig.MacFilterMode.DENY, Set.of(STA1)).build(), 81);
        StationCore denied = air.addStation(STA1, 82);
        StationCore ok = air.addStation(STA2, 83);
        air.advance(110);
        assertTrue(denied.connect(SSID, PASS, air.now));
        air.pump();
        assertEquals(StationCore.State.IDLE, denied.state());
        assertEquals("authentication rejected (status 1)", denied.lastError());
        assertNull(air.ap.client(STA1));
        air.join(ok, SSID, PASS);
        assertEquals(StationCore.State.CONNECTED, ok.state());

        // An open-only station cannot associate to the WPA2 BSS (no RSN element): status 40.
        IdealAir air2 = new IdealAir(wpa2().build(), 84);
        air2.addStation(STA1, 85);
        air2.advance(110);
        air2.ap.onReceive(Frames.mgmt(FrameControl.AUTH, BSSID, STA1, BSSID, 0,
                new Mgmt.Auth(0, 1, 0, List.of()).encode()), RxMeta.NONE, air2.now);
        air2.pump();
        int at = air2.log.size();
        air2.ap.onReceive(Frames.mgmt(FrameControl.ASSOC_REQ, BSSID, STA1, BSSID, 1,
                new Mgmt.AssocRequest(1, 10, null, List.of(Ie.ssid(
                        SSID.getBytes(StandardCharsets.UTF_8)))).encode()), RxMeta.NONE, air2.now);
        air2.pump();
        Mgmt.AssocResponse resp = Mgmt.AssocResponse.parse(Frame80211.parse(air2.log.get(at).frame()).body());
        assertEquals(Mgmt.STATUS_INVALID_IE, resp.status());
    }

    @Test
    void beaconsEvery102point4MsAndRunsAreDeterministic() {
        IdealAir air = new IdealAir(wpa2().build(), 91);
        air.ap.tick(0);
        for (int t = 1; t < 1024; t++) air.ap.tick(t);
        assertEquals(10, air.ap.beaconsSent()); // t = 0, 102.4, ..., 921.6 ms
        air.ap.tick(1024);
        assertEquals(11, air.ap.beaconsSent());

        byte[][] a = sessionBytes();
        byte[][] b = sessionBytes();
        assertEquals(a.length, b.length);
        for (int i = 0; i < a.length; i++) assertArrayEquals(a[i], b[i], "frame " + i);
    }

    private static byte[][] sessionBytes() {
        IdealAir air = new IdealAir(wpa2().build(), 5);
        StationCore s = air.addStation(STA1, 6);
        air.join(s, SSID, PASS);
        air.send(s, WIRED_HOST, payload("deterministic"));
        return air.log.stream().map(IdealAir.Tx::frame).toArray(byte[][]::new);
    }

    @Test
    void malformedFramesAreCountedNotThrown() {
        IdealAir air = new IdealAir(wpa2().build(), 101);
        air.ap.onReceive(new byte[5], RxMeta.NONE, 0);
        byte[] badIe = Frames.mgmt(FrameControl.PROBE_REQ, MacAddress.BROADCAST, STA1, MacAddress.BROADCAST, 0, new byte[]{0, 9, 1});
        air.ap.onReceive(badIe, RxMeta.NONE, 0);
        air.ap.onWiredFrame(new byte[3]);
        assertEquals(3, air.ap.malformedFrames());
        // A frame on another channel is ignored by the AP.
        air.ap.onReceive(Frames.mgmt(FrameControl.PROBE_REQ, MacAddress.BROADCAST, STA1, MacAddress.BROADCAST, 0,
                new byte[]{0, 0}), new RxMeta(-40, 1000, 1), 0);
        air.pump();
        assertEquals(0, air.log.size());
    }
}
