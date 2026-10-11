package com.example.evanscomputermod.radio.wifi.ap;

import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.wifi80211.MacAddress;
import com.example.evanscomputermod.radio.wifi80211.sta.StaOutput;
import com.example.evanscomputermod.radio.wifi80211.sta.StationCore;

import java.util.ArrayList;
import java.util.List;
import java.util.Random;
import java.util.UUID;
import java.util.concurrent.CopyOnWriteArrayList;

/**
 * A Wi-Fi client with no computer behind it: a {@link StationCore} on a
 * {@link WifiAirLink} at a fixed pose, joining a network by SSID and
 * passphrase (rejoining on its own after a failure), with an optional tiny
 * IPv4 host ({@link IpResponder}) so it answers ARP and ping. Scenarios use it
 * as "a phone in the room" and GameTests use it to prove the Access Point
 * end to end before the Wi-Fi module exists. Pure; tick it from one thread.
 */
public final class VirtualStation {

    /** Rejoin attempts are this far apart. */
    public static final long RETRY_MS = 2_000;

    private final WifiAirLink link;
    private final StationCore core;
    private final List<byte[]> delivered = new CopyOnWriteArrayList<>();
    private final List<byte[]> pendingTx = new ArrayList<>();
    private final List<byte[]> pendingReplies = new ArrayList<>();
    private final String ssid;
    private final String passphrase;
    private int ip;
    private long nextAttemptMs;
    private long answered;
    private boolean everConnected;

    /**
     * @param passphrase null for an open network
     */
    public VirtualStation(MacAddress mac, Pose pose, int wifiChannel, String ssid, String passphrase, long seed) {
        this.ssid = ssid;
        this.passphrase = passphrase;
        this.link = new WifiAirLink(UUID.nameUUIDFromBytes(("ecm-virtual-sta-" + mac).getBytes()),
                ApAntenna.INSTANCE, 20, wifiChannel, 15);
        this.link.setPose(pose);
        this.link.setAckAddress(mac);
        this.core = new StationCore(mac, new StaOutput() {
            @Override public void transmitRadio(byte[] f) { pendingTx.add(f); }
            @Override public void deliverEthernet(byte[] eth) { onEthernet(eth); }
        }, new Random(seed));
    }

    /** Answer ARP and ICMP echo for this IPv4 address (0 = don't). */
    public VirtualStation withIp(int ipv4) {
        this.ip = ipv4;
        return this;
    }

    public void attach(RadioMedium medium) {
        link.attach(medium);
    }

    public void detach() {
        if (core.state() != StationCore.State.IDLE) {
            core.disconnect(3, nowMs());
            flush();
        }
        link.attach(null);
    }

    private long nowMs() {
        RadioMedium m = link.medium();
        return m == null ? 0 : m.nowMicros() / 1000;
    }

    /** Receive what arrived, run timers, (re)join, transmit. */
    public void tick() {
        long now = nowMs();
        link.drain((f, meta) -> core.onReceive(f, meta, now));
        for (byte[] reply : pendingReplies) if (core.sendEthernet(reply, now)) answered++;
        pendingReplies.clear();
        core.tick(now);
        if (core.state() == StationCore.State.CONNECTED) everConnected = true;
        if (core.state() == StationCore.State.IDLE && now >= nextAttemptMs) {
            nextAttemptMs = now + RETRY_MS;
            if (!core.connect(ssid, passphrase, now)) core.probe(ssid, link.wifiChannel(), now);
        }
        flush();
    }

    /** Sends an Ethernet frame through the AP; false if not connected. */
    public boolean sendEthernet(byte[] eth) {
        boolean ok = core.sendEthernet(eth, nowMs());
        flush();
        return ok;
    }

    private void onEthernet(byte[] eth) {
        delivered.add(eth.clone());
        if (ip != 0) {
            byte[] reply = IpResponder.respond(eth, core.mac(), ip);
            if (reply != null) pendingReplies.add(reply);   // sent after the receive loop, never re-entrantly
        }
    }

    private void flush() {
        for (byte[] f : pendingTx) link.send(f);
        pendingTx.clear();
    }

    public MacAddress mac() { return core.mac(); }
    public StationCore core() { return core; }
    public WifiAirLink link() { return link; }
    public boolean connected() { return core.state() == StationCore.State.CONNECTED; }
    public boolean everConnected() { return everConnected; }
    /** Every Ethernet frame the station received (decrypted), oldest first. */
    public List<byte[]> delivered() { return delivered; }
    public long answered() { return answered; }
    public String ssid() { return ssid; }
}
