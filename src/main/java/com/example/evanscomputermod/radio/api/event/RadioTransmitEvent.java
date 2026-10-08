package com.example.evanscomputermod.radio.api.event;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Pose;
import net.neoforged.bus.api.Event;
import net.neoforged.bus.api.ICancellableEvent;

import java.util.UUID;

/**
 * Fired on the NeoForge event bus before player-controlled hardware keys up a
 * transmitter (SDR transmit, amplifiers): who, where, which channel and at what
 * power. Cancel it to block the transmission (claims, server rules, KubeJS via
 * NativeEvents). Not fired per Wi-Fi/controller frame (those are milliwatt
 * packet radios and would flood the bus).
 *
 * <p>May be fired from a computer's program thread; listeners must not touch
 * the world directly.
 */
public class RadioTransmitEvent extends Event implements ICancellableEvent {
    private final UUID source;
    private final Pose pose;
    private final Channel channel;
    private final double powerDbm;
    private final String kind;

    public RadioTransmitEvent(UUID source, Pose pose, Channel channel, double powerDbm, String kind) {
        this.source = source;
        this.pose = pose;
        this.channel = channel;
        this.powerDbm = powerDbm;
        this.kind = kind;
    }

    public UUID source() { return source; }
    public Pose pose() { return pose; }
    public Channel channel() { return channel; }
    public double powerDbm() { return powerDbm; }
    public double powerWatts() { return Math.pow(10, powerDbm / 10) / 1000; }
    /** "sdr", "amplifier", ... */
    public String kind() { return kind; }
}
//?}
