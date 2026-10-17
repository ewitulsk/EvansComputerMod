package com.example.evanscomputermod.bluetooth;

import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;

import java.util.UUID;
import java.util.concurrent.ConcurrentLinkedQueue;

/**
 * A Bluetooth radio on the medium: a small chip antenna at a (moving) place, tuned
 * to one 2 MHz channel, transmitting 2 Mb/s frames at {@link #TX_POWER_DBM}.
 * Frames it hears wait in {@link #inbox} for the owner's server tick.
 */
public final class BtEndpoint implements RadioEndpoint {

    /** Class 2 Bluetooth: 4 dBm. */
    public static final double TX_POWER_DBM = 4;
    public static final double BIT_RATE = 2e6;
    public static final String MODULATION = "BT-EDR2";

    /** A chip antenna, nearly omnidirectional, vertically polarized like the phone's other radios. */
    public static final AntennaPattern CHIP = new AntennaPattern() {
        @Override public double gainDbi(double lx, double ly, double lz) { return -1; }
        @Override public double[] polarization(double lx, double ly, double lz) { return new double[] {0, 1, 0}; }
        @Override public double peakGainDbi() { return -1; }
    };

    /** A frame as heard. */
    public record Heard(BtFrame frame, double rssiDbm) {}

    private final UUID id;
    private volatile Pose pose = Pose.at("minecraft:overworld", 0, -10_000, 0);
    private volatile Channel tuned;
    private final ConcurrentLinkedQueue<Heard> inbox = new ConcurrentLinkedQueue<>();
    private RadioMedium medium;

    public BtEndpoint(UUID id) {
        this.id = id;
    }

    /** Keep registered with the server's medium (it changes across server restarts); null unregisters. */
    public void sync(RadioMedium m) {
        if (m == medium) return;
        if (medium != null) medium.unregister(this);
        medium = m;
        if (m != null) m.register(this);
    }

    public void close() {
        sync(null);
        inbox.clear();
    }

    public void moveTo(Pose p) {
        if (p != null) pose = p;
    }

    public void tune(Channel c) {
        if (java.util.Objects.equals(c, tuned)) return;
        tuned = c;
        if (medium != null) medium.invalidate(this);
    }

    /** Put a frame on the air on {@code channel}. */
    public void transmit(BtFrame f, Channel channel) {
        RadioMedium m = medium;
        if (m == null || channel == null) return;
        byte[] bytes = f.encode();
        m.transmit(this, Emission.frame(channel, TX_POWER_DBM, m.nowMicros(), BtFrame.airtimeMicros(bytes.length), MODULATION, BIT_RATE, bytes));
    }

    /** The frames heard since the last call. */
    public Heard poll() {
        return inbox.poll();
    }

    @Override public UUID id() { return id; }
    @Override public Pose pose() { return pose; }
    @Override public AntennaPattern antenna() { return CHIP; }
    @Override public Channel tunedChannel() { return tuned; }
    @Override public double maxTxPowerDbm() { return TX_POWER_DBM; }
    @Override public double sensitivityDbm() { return -90; }
    @Override public double speedMps() { return 1.5; }

    @Override
    public void onReceive(Reception r) {
        BtFrame f = BtFrame.decode(r.payload());
        if (f != null && inbox.size() < 512) inbox.offer(new Heard(f, r.rssiDbm()));
    }
}
