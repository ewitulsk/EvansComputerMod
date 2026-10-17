package com.example.evanscomputermod.bluetooth;

import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.speaker.ImaAdpcm;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.function.Supplier;

/**
 * A phone's Bluetooth controller (docs/BLUETOOTH.md): it listens on the advertising
 * channel for devices, pairs with one when asked (sending it a fresh link key),
 * connects to paired devices that advertise themselves as connectable, and keeps a
 * link on a data channel that carries the phone's audio out and the headset's
 * microphone back. Every frame goes over the radio medium, so range, walls and
 * other 2.4 GHz traffic apply. Pure Java (the radio is behind {@link Radio}), so
 * the protocol is unit tested; {@code PhoneBluetooth} wires it into the ePhone.
 */
public final class PhoneLink {

    /** How frames reach the air. */
    public interface Radio {
        /** Transmit on the advertising channel ({@code data == null}) or on a data channel. */
        void send(BtFrame frame, Channel data);

        /** Listen on this data channel (null: no link). */
        void tuneData(Channel data);
    }

    /** A device this phone paired with. */
    public record Paired(long addr, String name, byte[] key) {}

    /** A device heard advertising. */
    public record Seen(long addr, String name, double rssiDbm, BtFrame.Adv adv, long atMs) {}

    /** The connected device. */
    public static final class Link {
        public final long addr;
        public final int channel;
        long lastRxMs;
        long lastCtrlMs;
        int seq;
        public volatile int battery = -1;
        public volatile boolean worn = true;
        public volatile double rssiDbm = Double.NaN;
        public volatile long audioFrames, micFrames;

        Link(long addr, int channel, long now) {
            this.addr = addr;
            this.channel = channel;
            this.lastRxMs = now;
        }
    }

    /** A link with nothing heard for this long is gone (link supervision). */
    public static final long SUPERVISION_MS = 2000;
    /** Keep-alive (CTRL) period. */
    public static final long CTRL_MS = 400;
    /** A device not heard advertising for this long leaves the scan list. */
    public static final long SEEN_MS = 4000;
    /** A pairing or connection request is repeated this often, and given up after PAIR_TIMEOUT_MS. */
    public static final long RETRY_MS = 300, PAIR_TIMEOUT_MS = 5000;

    public final long addr;
    private final Radio radio;
    private final Supplier<byte[]> keys;
    private String name;
    private boolean powered = true;
    private final Map<Long, Paired> paired = new LinkedHashMap<>();
    private final Map<Long, Seen> seen = new LinkedHashMap<>();
    private long pairing = -1, pairingSince, pairingSent;
    private byte[] pairingKey;
    private long connecting = -1, connectingSince, connectSent;
    /** Don't reconnect to this device on its own (the player disconnected it), until it is connected by hand. */
    private long stayOff = -1;
    private Link link;
    private final ImaAdpcm encoder = new ImaAdpcm();
    private String lastResult = "";
    private boolean changed;

    /** Where the headset's microphone goes: samples at a rate. */
    public interface MicSink {
        void accept(short[] pcm, int rate);
    }

    private MicSink micSink = (pcm, rate) -> {};

    public PhoneLink(long addr, String name, Radio radio, Supplier<byte[]> keys) {
        this.addr = addr;
        this.name = name;
        this.radio = radio;
        this.keys = keys;
    }

    public void setMicSink(MicSink sink) {
        micSink = sink;
    }

    public void setName(String n) {
        name = n;
    }

    public String name() {
        return name;
    }

    // ------------------------------------------------------------------ state

    public synchronized boolean powered() {
        return powered;
    }

    public synchronized void setPowered(boolean on) {
        if (powered == on) return;
        powered = on;
        if (!on) {
            drop(true);
            seen.clear();
            pairing = -1;
            connecting = -1;
        }
        changed = true;
    }

    public synchronized List<Paired> paired() {
        return new ArrayList<>(paired.values());
    }

    public synchronized void restore(List<Paired> list) {
        paired.clear();
        for (Paired p : list) paired.put(p.addr(), p);
    }

    /** Devices heard advertising lately, strongest first. */
    public synchronized List<Seen> seen(long now) {
        seen.values().removeIf(s -> now - s.atMs() > SEEN_MS);
        List<Seen> out = new ArrayList<>(seen.values());
        out.sort((a, b) -> Double.compare(b.rssiDbm(), a.rssiDbm()));
        return out;
    }

    public synchronized Link link() {
        return link;
    }

    /** The connected device's address, or -1. */
    public synchronized long connected() {
        return link == null ? -1 : link.addr;
    }

    /** The phone's audio goes to the headset (connected and in the player's ears). */
    public synchronized boolean audioRouted() {
        return link != null && link.worn;
    }

    /** What happened to the last pair/connect request, for the Settings app. */
    public synchronized String lastResult() {
        return lastResult;
    }

    public synchronized boolean pairingWith(long a) {
        return pairing == a;
    }

    public synchronized boolean connectingTo(long a) {
        return connecting == a;
    }

    /** True once since the last call if the paired list or power changed (the owner saves). */
    public synchronized boolean takeChanged() {
        boolean c = changed;
        changed = false;
        return c;
    }

    // ------------------------------------------------------------------ requests

    /** Pair with a device that advertises as pairable. */
    public synchronized String pair(long a, long now) {
        if (!powered) return "Bluetooth is off";
        Seen s = seen.get(a);
        if (s == null || now - s.atMs() > SEEN_MS) return "not in range: open its case lid next to the phone";
        if (!s.adv().pairable()) return "it isn't in pairing mode: open its case lid";
        pairing = a;
        pairingKey = keys.get();
        pairingSince = now;
        pairingSent = 0;
        lastResult = "pairing";
        sendPairRequest(now);
        return null;
    }

    /** Forget a paired device (disconnecting it). */
    public synchronized void forget(long a) {
        if (link != null && link.addr == a) {
            radio.send(new BtFrame(BtFrame.Type.DISCONNECT, addr, a, new byte[0]), BtAddress.data(link.channel));
            drop(false);
        }
        if (paired.remove(a) != null) changed = true;
        if (connecting == a) connecting = -1;
    }

    /** Connect to a paired device now (it must be advertising: worn, or its case open). */
    public synchronized String connect(long a, long now) {
        if (!powered) return "Bluetooth is off";
        if (!paired.containsKey(a)) return "not paired";
        if (link != null && link.addr == a) return null;
        stayOff = -1;
        connecting = a;
        connectingSince = now;
        connectSent = 0;
        lastResult = "connecting";
        return null;
    }

    /** Hang up the link (it stays paired; it won't reconnect by itself until connected again). */
    public synchronized void disconnect() {
        if (link == null) return;
        stayOff = link.addr;
        radio.send(new BtFrame(BtFrame.Type.DISCONNECT, addr, link.addr, new byte[0]), BtAddress.data(link.channel));
        drop(false);
    }

    private void drop(boolean notify) {
        if (link != null && notify) {
            radio.send(new BtFrame(BtFrame.Type.DISCONNECT, addr, link.addr, new byte[0]), BtAddress.data(link.channel));
        }
        link = null;
        radio.tuneData(null);
    }

    private void sendPairRequest(long now) {
        pairingSent = now;
        radio.send(BtFrame.pairRequest(addr, pairing, pairingKey, name), null);
    }

    private void sendConnect(long now) {
        Paired p = paired.get(connecting);
        if (p == null) {
            connecting = -1;
            return;
        }
        connectSent = now;
        radio.send(BtFrame.connectRequest(addr, connecting, BtAddress.channelFor(addr, connecting), p.key()), null);
    }

    // ------------------------------------------------------------------ the air

    /** A frame heard on either channel. */
    public synchronized void receive(BtFrame f, double rssiDbm, long now) {
        if (!powered || f.from() == addr) return;
        switch (f.type()) {
            case ADV -> {
                BtFrame.Adv a = f.adv();
                if (a == null) return;
                seen.put(f.from(), new Seen(f.from(), a.name(), rssiDbm, a, now));
                // A paired device that wants its phone back (worn, or its case opened): reconnect.
                if (a.connectable() && a.pairedTo() == addr && paired.containsKey(f.from()) && link == null
                        && connecting < 0 && stayOff != f.from()) {
                    connecting = f.from();
                    connectingSince = now;
                    connectSent = 0;
                }
                if (link == null && connecting == f.from() && now - connectSent >= RETRY_MS) sendConnect(now);
            }
            case PAIR_RSP -> {
                if (!f.isFor(addr) || f.from() != pairing) return;
                if (f.ok()) {
                    Seen s = seen.get(pairing);
                    paired.put(pairing, new Paired(pairing, s == null ? BtAddress.format(pairing) : s.name(), pairingKey));
                    changed = true;
                    lastResult = "paired";
                    connecting = pairing;
                    connectingSince = now;
                    connectSent = 0;
                    stayOff = -1;
                } else {
                    lastResult = "refused";
                }
                pairing = -1;
            }
            case CONNECT_RSP -> {
                if (!f.isFor(addr) || f.from() != connecting) return;
                if (f.ok()) {
                    link = new Link(f.from(), BtAddress.channelFor(addr, f.from()), now);
                    link.rssiDbm = rssiDbm;
                    radio.tuneData(BtAddress.data(link.channel));
                    lastResult = "connected";
                } else {
                    lastResult = "refused";
                }
                connecting = -1;
            }
            case MIC -> {
                if (link == null || f.from() != link.addr || !f.isFor(addr)) return;
                link.lastRxMs = now;
                link.rssiDbm = rssiDbm;
                BtFrame.Audio a = f.audio();
                if (a == null) return;
                short[] pcm = ImaAdpcm.decode(a.adpcm(), a.samples());
                if (pcm != null) {
                    link.micFrames++;
                    micSink.accept(pcm, a.rate());
                }
            }
            case CTRL -> {
                if (link == null || f.from() != link.addr) return;
                link.lastRxMs = now;
                link.rssiDbm = rssiDbm;
                BtFrame.Ctrl c = f.ctrl();
                if (c != null) {
                    link.battery = c.battery();
                    link.worn = c.worn();
                }
            }
            case DISCONNECT -> {
                if (link != null && f.from() == link.addr && f.isFor(addr)) drop(false);
            }
            default -> {}
        }
    }

    /** Every server tick: retries, keep-alives, link supervision. */
    public synchronized void tick(long now, boolean micWanted) {
        if (!powered) return;
        if (pairing >= 0) {
            if (now - pairingSince > PAIR_TIMEOUT_MS) {
                pairing = -1;
                lastResult = "no answer: open its case lid next to the phone and try again";
            } else if (now - pairingSent >= RETRY_MS) {
                sendPairRequest(now);
            }
        }
        if (connecting >= 0 && link == null) {
            if (now - connectingSince > PAIR_TIMEOUT_MS) {
                connecting = -1;
                lastResult = "no answer: put the AirPods in your ears or open their case";
            } else if (now - connectSent >= RETRY_MS) {
                sendConnect(now);
            }
        }
        if (link != null) {
            if (now - link.lastRxMs > SUPERVISION_MS) {
                lastResult = "lost the connection (out of range?)";
                drop(false);
                return;
            }
            if (now - link.lastCtrlMs >= CTRL_MS) {
                link.lastCtrlMs = now;
                radio.send(BtFrame.ctrl(addr, link.addr, new BtFrame.Ctrl(micWanted, 100, true)), BtAddress.data(link.channel));
            }
        }
    }

    /** The phone played this: send it to the headset (in blocks that fit a frame). */
    public synchronized void sendAudio(short[] pcm, int rate) {
        if (link == null || pcm.length == 0) return;
        for (int off = 0; off < pcm.length; off += 1600) {
            int n = Math.min(1600, pcm.length - off);
            byte[] block = encoder.encode(pcm, off, n);
            link.audioFrames++;
            radio.send(BtFrame.audio(BtFrame.Type.AUDIO, addr, link.addr, new BtFrame.Audio(link.seq++ & 0xffff, rate, n, block)),
                    BtAddress.data(link.channel));
        }
    }
}
