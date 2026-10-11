package com.example.evanscomputermod.radio.wifi.ap;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.medium.BasicRadioMedium;
import com.example.evanscomputermod.radio.wifi80211.MacAddress;
import com.example.evanscomputermod.radio.wifi80211.Security;
import com.example.evanscomputermod.radio.wifi80211.ap.AccessPointCore;
import com.example.evanscomputermod.radio.wifi80211.ap.ApOutput;
import com.example.evanscomputermod.radio.wifi80211.ap.ClientStatus;
import com.example.evanscomputermod.radio.wifi80211.frame.EthernetFrame;

import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.List;
import java.util.Random;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicLong;

/**
 * The AP block's radio plumbing without Minecraft: an {@link AccessPointCore}
 * on a {@link WifiAirLink} and a {@link VirtualStation}, both on the reference
 * {@link BasicRadioMedium} with a virtual clock. Proves association + WPA2
 * over the medium, bridging both ways, rates/airtime on the emissions, and the
 * wrong-passphrase and out-of-range controls.
 */
public class AccessPointOverMediumTest {

    static final MacAddress BSSID = MacAddress.parse("02:a1:00:00:00:01");
    static final MacAddress STA = MacAddress.parse("02:5a:00:00:00:02");
    static final MacAddress WIRED = MacAddress.parse("02:00:00:00:00:99");

    final AtomicLong clock = new AtomicLong(1_000_000);
    final List<Emission> onAir = new ArrayList<>();
    final BasicRadioMedium medium = new BasicRadioMedium(clock::get, 42) {
        @Override
        public Emission transmit(RadioEndpoint from, Emission e) {
            Emission s = super.transmit(from, e);
            onAir.add(s);
            return s;
        }
    };
    final List<byte[]> wiredOut = new ArrayList<>();
    final List<MacAddress> removed = new ArrayList<>();
    final List<byte[]> radioOut = new ArrayList<>();
    AccessPointCore core;
    WifiAirLink apLink;

    void startAp(Security sec, String pass) {
        ApSettings s = new ApSettings("Lab", false, sec, 6, 20, false, null, null);
        core = new AccessPointCore(s.toConfig(BSSID, 6, pass), new ApOutput() {
            @Override public void transmitRadio(byte[] f) { radioOut.add(f); }
            @Override public void transmitWired(byte[] eth, MacAddress src) { wiredOut.add(eth); }
            @Override public void clientRemoved(MacAddress mac) { removed.add(mac); }
        }, new Random(7));
        apLink = new WifiAirLink(UUID.randomUUID(), ApAntenna.INSTANCE, 20, 6, 20);
        apLink.setPose(Pose.at("overworld", 0, 64, 0));
        apLink.attach(medium);
    }

    void flush() {
        for (byte[] f : new ArrayList<>(radioOut)) apLink.send(f);
        radioOut.clear();
    }

    void run(long ms, VirtualStation... stas) {
        for (long i = 0; i < ms; i++) {
            clock.addAndGet(1000);
            long now = clock.get() / 1000;
            apLink.drain((f, meta) -> core.onReceive(f, meta, now));
            flush();
            core.tick(now);
            flush();
            for (VirtualStation s : stas) s.tick();
        }
    }

    VirtualStation station(MacAddress mac, double x, String pass) {
        VirtualStation s = new VirtualStation(mac, Pose.at("overworld", x, 64, 0), 6, "Lab", pass, 3).withIp(0x0a000514);
        s.attach(medium);
        return s;
    }

    @Test
    void wpa2StationJoinsAndBridgesBothWays() {
        startAp(Security.WPA2_PSK, "correct horse battery");
        VirtualStation sta = station(STA, 15, "correct horse battery");
        run(1500, sta);
        assertTrue(sta.connected(), "station state " + sta.core().state() + " / " + sta.core().lastError());
        ClientStatus c = core.client(STA);
        assertEquals(ClientStatus.State.AUTHORIZED, c.state());
        assertEquals(ClientStatus.Handshake.DONE, c.handshake());
        assertTrue(c.lastRssiDbm() < -30 && c.lastRssiDbm() > -70, "rssi " + c.lastRssiDbm());

        // Uplink broadcast reaches the cable with the station's source MAC.
        byte[] arp = new EthernetFrame(MacAddress.BROADCAST, STA, EthernetFrame.ETHERTYPE_ARP, new byte[28]).encode();
        assertTrue(sta.sendEthernet(arp));
        run(5, sta);
        assertEquals(1, wiredOut.size());
        assertEquals(STA, MacAddress.read(wiredOut.get(0), 6));

        // Downlink unicast arrives decrypted at the station; on air it was protected and at an OFDM rate.
        int before = sta.delivered().size();
        onAir.clear();
        core.onWiredFrame(new EthernetFrame(STA, WIRED, EthernetFrame.ETHERTYPE_IPV4, "hello".getBytes()).encode());
        flush();
        run(3, sta);
        assertEquals(before + 1, sta.delivered().size());
        assertEquals(WIRED, MacAddress.read(sta.delivered().get(before), 6));
        Emission data = onAir.stream().filter(e -> e.payload().length > 24 && (e.payload()[0] & 0x0C) == 0x08).findFirst().orElseThrow();
        assertTrue((data.payload()[1] & 0x40) != 0, "data frame not protected");
        assertTrue(data.modulation().startsWith("OFDM-"), data.modulation());
        assertEquals(ApRadioPlan.airtimeUs(new ApRadioPlan.Rate(data.modulation(), (int) (data.bitRate() / 1000)), data.payload().length, 6),
                data.durationMicros());

        // Beacons go out at 1 Mbit/s DSSS, about every 102.4 ms.
        onAir.clear();
        run(1024);
        List<Emission> bs = onAir.stream().filter(e -> (e.payload()[0] & 0xFC) == 0x80).toList();
        assertTrue(bs.size() >= 9 && bs.size() <= 11, "beacons in 1.024 s: " + bs.size());
        assertEquals("DSSS-1", bs.get(0).modulation());
    }

    @Test
    void wrongPassphraseNeverAuthorizesNorBridges() {
        startAp(Security.WPA2_PSK, "correct horse battery");
        VirtualStation sta = station(STA, 15, "wrong horse battery");
        run(3000, sta);
        assertFalse(sta.everConnected(), "connected with the wrong passphrase");
        ClientStatus c = core.client(STA);
        assertTrue(c == null || c.state() != ClientStatus.State.AUTHORIZED);
        assertFalse(sta.sendEthernet(new EthernetFrame(MacAddress.BROADCAST, STA, 0x0806, new byte[28]).encode()));
        assertTrue(wiredOut.isEmpty(), "frames bridged without authorization");
        boolean micFailure = core.recentEvents().stream().anyMatch(e -> e.toLowerCase().contains("mic"));
        assertTrue(micFailure, "no MIC failure logged: " + core.recentEvents());
    }

    @Test
    void openNetworkWorksAndDistanceIsAControl() {
        startAp(Security.OPEN, null);
        VirtualStation near = station(STA, 20, null);
        VirtualStation far = station(MacAddress.parse("02:5a:00:00:00:03"), 200_000, null);
        run(1500, near, far);
        assertTrue(near.connected(), "near open station: " + near.core().lastError());
        assertFalse(far.everConnected(), "a station 200 km away associated");
        assertTrue(near.sendEthernet(new EthernetFrame(MacAddress.BROADCAST, STA, 0x0806, new byte[28]).encode()));
        run(3, near, far);
        assertEquals(1, wiredOut.size());
    }
}
