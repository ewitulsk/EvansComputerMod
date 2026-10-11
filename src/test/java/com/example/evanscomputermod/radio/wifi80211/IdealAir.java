package com.example.evanscomputermod.radio.wifi80211;

import com.example.evanscomputermod.radio.wifi80211.ap.AccessPointCore;
import com.example.evanscomputermod.radio.wifi80211.ap.ApConfig;
import com.example.evanscomputermod.radio.wifi80211.ap.ApOutput;
import com.example.evanscomputermod.radio.wifi80211.frame.EapolKey;
import com.example.evanscomputermod.radio.wifi80211.frame.EthernetFrame;
import com.example.evanscomputermod.radio.wifi80211.frame.Frame80211;
import com.example.evanscomputermod.radio.wifi80211.frame.FrameControl;
import com.example.evanscomputermod.radio.wifi80211.frame.Llc;
import com.example.evanscomputermod.radio.wifi80211.frame.PcapWriter;
import com.example.evanscomputermod.radio.wifi80211.frame.Radiotap;
import com.example.evanscomputermod.radio.wifi80211.sta.StaOutput;
import com.example.evanscomputermod.radio.wifi80211.sta.StationCore;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Deque;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Random;
import java.util.function.BiPredicate;

/**
 * Lossless, zero-latency radio medium for one AP and some stations, with a
 * virtual clock, a drop hook and a radiotap pcap of everything on the air.
 */
final class IdealAir {

    record Tx(Object from, byte[] frame) {}

    record WiredTx(byte[] eth, MacAddress src) {}

    final AccessPointCore ap;
    final Map<MacAddress, StationCore> stations = new LinkedHashMap<>();
    final Map<MacAddress, List<byte[]>> delivered = new LinkedHashMap<>();
    final List<WiredTx> wired = new ArrayList<>();
    final List<Tx> log = new ArrayList<>();
    final List<MacAddress> authorized = new ArrayList<>();
    final List<MacAddress> removed = new ArrayList<>();
    final PcapWriter pcap = new PcapWriter(PcapWriter.LINKTYPE_IEEE802_11_RADIO);
    private final Deque<Tx> queue = new ArrayDeque<>();
    private final int channel;
    /** Return true to drop a frame (sender, frame). */
    BiPredicate<Object, byte[]> drop = (from, f) -> false;
    long now;

    IdealAir(ApConfig cfg, long seed) {
        channel = cfg.channel();
        ap = new AccessPointCore(cfg, new ApOutput() {
            @Override public void transmitRadio(byte[] f) { queue.add(new Tx(ap(), f)); }
            @Override public void transmitWired(byte[] eth, MacAddress src) { wired.add(new WiredTx(eth, src)); }
            @Override public void clientAuthorized(MacAddress mac) { authorized.add(mac); }
            @Override public void clientRemoved(MacAddress mac) { removed.add(mac); }
        }, new Random(seed));
    }

    private Object ap() {
        return ap;
    }

    StationCore addStation(MacAddress mac, long seed) {
        List<byte[]> rx = new ArrayList<>();
        delivered.put(mac, rx);
        StationCore[] holder = new StationCore[1];
        holder[0] = new StationCore(mac, new StaOutput() {
            @Override public void transmitRadio(byte[] f) { queue.add(new Tx(holder[0], f)); }
            @Override public void deliverEthernet(byte[] eth) { rx.add(eth); }
        }, new Random(seed));
        stations.put(mac, holder[0]);
        return holder[0];
    }

    /** Delivers queued frames (and whatever they trigger) until the air is quiet. */
    void pump() {
        RxMeta meta = new RxMeta(-48, 54_000, channel);
        int guard = 0;
        while (!queue.isEmpty()) {
            if (++guard > 100_000) throw new IllegalStateException("frame storm");
            Tx tx = queue.poll();
            log.add(tx);
            pcap.add(now * 1000, Radiotap.wrap(tx.frame(), 54_000, channel, -48, false));
            if (drop.test(tx.from(), tx.frame())) continue;
            if (tx.from() != ap) ap.onReceive(tx.frame(), meta, now);
            for (StationCore s : stations.values()) {
                if (s != tx.from()) s.onReceive(tx.frame(), meta, now);
            }
        }
    }

    /** Advances the clock in 1 ms steps, ticking everyone and pumping the air. */
    void advance(long ms) {
        for (long i = 0; i < ms; i++) {
            now++;
            ap.tick(now);
            for (StationCore s : stations.values()) s.tick(now);
            pump();
        }
    }

    /** Scan (beacons for a moment), then connect and run until the handshake settles. */
    void join(StationCore s, String ssid, String pass) {
        advance(110);
        if (!s.connect(ssid, pass, now)) throw new AssertionError("connect failed: " + s.lastError());
        pump();
        advance(5);
    }

    void wire(MacAddress dst, MacAddress src, int type, byte[] payload) {
        ap.onWiredFrame(new EthernetFrame(dst, src, type, payload).encode());
        pump();
    }

    void send(StationCore s, MacAddress dst, byte[] payload) {
        if (!s.sendEthernet(new EthernetFrame(dst, s.mac(), EthernetFrame.ETHERTYPE_IPV4, payload).encode(), now)) {
            throw new AssertionError("station not connected: " + s.state());
        }
        pump();
    }

    /** Key Information of a plaintext EAPOL-Key frame, or -1. */
    static int eapolKeyInfo(byte[] frame) {
        try {
            Frame80211 f = Frame80211.parse(frame);
            if (f.header().type() != FrameControl.TYPE_DATA || f.header().isProtected()) return -1;
            Llc.Decap d = Llc.decap(f.body());
            if (d.etherType() != EthernetFrame.ETHERTYPE_EAPOL) return -1;
            return EapolKey.parse(d.payload()).keyInfo();
        } catch (RuntimeException e) {
            return -1;
        }
    }

    long countOnAir(int keyInfo) {
        return log.stream().filter(t -> eapolKeyInfo(t.frame()) == keyInfo).count();
    }

    static boolean isProtectedData(byte[] frame) {
        Frame80211 f = Frame80211.parse(frame);
        return f.header().type() == FrameControl.TYPE_DATA && f.header().isProtected();
    }

    static int subtypeOf(byte[] frame) {
        Frame80211 f = Frame80211.parse(frame);
        return f.header().type() * 16 + f.header().subtype();
    }
}
