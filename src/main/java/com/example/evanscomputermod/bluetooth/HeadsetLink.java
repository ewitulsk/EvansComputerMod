package com.example.evanscomputermod.bluetooth;

import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.speaker.ImaAdpcm;

/**
 * The AirPods' radio (docs/BLUETOOTH.md): they advertise while their case lid is
 * open (pairable) or while they are worn and paired (connectable, for their
 * phone), accept a pairing from a phone while pairable, accept their paired
 * phone's connection, play the audio it sends and send their microphone back
 * when the phone asks for it. One antenna: while connected it listens on the
 * link's data channel only. Pure Java (the radio is behind {@link Radio}); the
 * AirPods item wires it to the medium and the player.
 */
public final class HeadsetLink {

    public interface Radio {
        /** Transmit on the advertising channel ({@code data == null}) or a data channel. */
        void send(BtFrame frame, Channel data);

        /** Listen on this channel (the advertising one while not connected). */
        void tune(Channel channel);
    }

    /** What the headset does with what it receives. */
    public interface Owner {
        /** A phone paired with us: remember it (the item keeps it). */
        void paired(long phone, byte[] key, String phoneName);

        /** Audio from the phone, for the wearer's ears. */
        void play(short[] pcm, int rate);
    }

    public static final long ADV_MS = 200;

    public final long addr;
    private final String name;
    private final Radio radio;
    private final Owner owner;
    private long pairedTo;
    private byte[] key;
    private boolean pairable, worn;
    private int battery = 100, caseBattery = 255;
    private long phone = -1;
    private int channel = -1;
    private long lastRxMs, lastAdvMs = -ADV_MS, lastCtrlMs;
    private boolean micWanted;
    private int micSeq;
    private final ImaAdpcm micEncoder = new ImaAdpcm();
    public volatile long audioFrames;

    public HeadsetLink(long addr, String name, Radio radio, Owner owner) {
        this.addr = addr;
        this.name = name;
        this.radio = radio;
        this.owner = owner;
        radio.tune(BtAddress.ADV);
    }

    /** What the item remembers: the paired phone and the link key. */
    public synchronized void restore(long pairedTo, byte[] key) {
        this.pairedTo = pairedTo;
        this.key = key;
    }

    /** Case lid open (pairable), worn, battery levels: set every tick by the owner. */
    public synchronized void setState(boolean pairable, boolean worn, int battery, int caseBattery) {
        this.pairable = pairable;
        this.worn = worn;
        this.battery = battery;
        this.caseBattery = caseBattery;
    }

    public synchronized boolean connected() {
        return phone >= 0;
    }

    public synchronized long phone() {
        return phone;
    }

    public synchronized long pairedTo() {
        return pairedTo;
    }

    /** The phone wants the microphone. */
    public synchronized boolean micWanted() {
        return phone >= 0 && micWanted && worn;
    }

    private void drop() {
        phone = -1;
        channel = -1;
        micWanted = false;
        radio.tune(BtAddress.ADV);
    }

    /** The AirPods go off the air (back in a closed case): tell the phone, so it doesn't wait for the link to time out. */
    public synchronized void goodbye() {
        if (phone >= 0) {
            radio.send(new BtFrame(BtFrame.Type.DISCONNECT, addr, phone, new byte[0]), BtAddress.data(channel));
            drop();
        }
    }

    public synchronized void receive(BtFrame f, long now) {
        if (!f.isFor(addr) || f.from() == addr) return;
        switch (f.type()) {
            case PAIR_REQ -> {
                if (!pairable || f.to() != addr) return;
                byte[] k = f.pairKey();
                if (k == null) return;
                if (phone >= 0 && phone != f.from()) drop();
                pairedTo = f.from();
                key = k;
                owner.paired(pairedTo, key, f.pairName());
                radio.send(BtFrame.answer(BtFrame.Type.PAIR_RSP, addr, f.from(), true), null);
            }
            case CONNECT_REQ -> {
                if (f.to() != addr) return;
                boolean ok = f.from() == pairedTo && f.proves(key) && (worn || pairable);
                if (ok && phone != f.from()) {
                    phone = f.from();
                    channel = f.connectChannel();
                    lastRxMs = now;
                    lastCtrlMs = 0;
                }
                radio.send(BtFrame.answer(BtFrame.Type.CONNECT_RSP, addr, f.from(), ok), null);
                if (ok) radio.tune(BtAddress.data(channel));
            }
            case AUDIO -> {
                if (f.from() != phone) return;
                lastRxMs = now;
                BtFrame.Audio a = f.audio();
                if (a == null || !worn) return;
                short[] pcm = ImaAdpcm.decode(a.adpcm(), a.samples());
                if (pcm != null) {
                    audioFrames++;
                    owner.play(pcm, a.rate());
                }
            }
            case CTRL -> {
                if (f.from() != phone) return;
                lastRxMs = now;
                BtFrame.Ctrl c = f.ctrl();
                if (c != null) micWanted = c.micWanted();
            }
            case DISCONNECT -> {
                if (f.from() == phone) drop();
            }
            default -> {}
        }
    }

    /** Every server tick: advertise, keep the link alive, notice a lost link. */
    public synchronized void tick(long now) {
        if (phone >= 0) {
            if (now - lastRxMs > PhoneLink.SUPERVISION_MS || !worn && !pairable) {
                drop();
            } else if (now - lastCtrlMs >= PhoneLink.CTRL_MS) {
                lastCtrlMs = now;
                radio.send(BtFrame.ctrl(addr, phone, new BtFrame.Ctrl(micWanted, battery, worn)), BtAddress.data(channel));
            }
            if (phone >= 0) return;
        }
        boolean connectable = worn && pairedTo != 0;
        if ((pairable || connectable) && battery > 0 && now - lastAdvMs >= ADV_MS) {
            lastAdvMs = now;
            int flags = (pairable ? BtFrame.FLAG_PAIRABLE : 0) | (connectable || pairable && pairedTo != 0 ? BtFrame.FLAG_CONNECTABLE : 0);
            radio.send(BtFrame.adv(addr, new BtFrame.Adv(flags, BtFrame.CLASS_HEADSET, battery, caseBattery, pairedTo, name)), null);
        }
    }

    /** The wearer's microphone (what the AirPods heard): to the phone, when it wants it. */
    public synchronized void sendMic(short[] pcm, int rate) {
        if (!micWanted() || pcm.length == 0) return;
        for (int off = 0; off < pcm.length; off += 1600) {
            int n = Math.min(1600, pcm.length - off);
            radio.send(BtFrame.audio(BtFrame.Type.MIC, addr, phone, new BtFrame.Audio(micSeq++ & 0xffff, rate, n, micEncoder.encode(pcm, off, n))),
                    BtAddress.data(channel));
        }
    }
}
