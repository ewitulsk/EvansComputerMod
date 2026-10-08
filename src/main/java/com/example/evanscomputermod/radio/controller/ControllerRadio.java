package com.example.evanscomputermod.radio.controller;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.controller.ControllerPackets;
import com.example.evanscomputermod.controller.ControllerState;
import com.example.evanscomputermod.module.ModuleBays;
import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import net.minecraft.server.level.ServerPlayer;
import net.neoforged.neoforge.network.PacketDistributor;

import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * The Wireless Controller as a radio: each input report becomes a 0 dBm,
 * 1 Mb/s, 2 MHz-wide frame from the player's position on the paired
 * computer's receiver channel. Range is whatever the link budget gives — no
 * configured distance — and the reports count as 2.4 GHz interference to Wi-Fi.
 */
public final class ControllerRadio {

    public static final double TX_POWER_DBM = 0;

    public enum Result { SENT, NO_RECEIVER, NO_MEDIUM }

    private record Delivery(long atMs, double rssi) {}

    private static final Map<UUID, Delivery> lastDelivery = new ConcurrentHashMap<>();
    private static final Map<UUID, Transmitter> transmitters = new ConcurrentHashMap<>();

    private ControllerRadio() {}

    /** Find a receiver on the computer: a Controller Receiver module, or a Wi-Fi module in controller mode. */
    public static ControllerReceiver receiverOf(TerminalBlockEntity tbe) {
        ModuleBays bays = tbe.getModuleBays();
        for (int slot = 0; slot < ModuleBays.SLOTS; slot++) {
            if (bays.getModule(slot) instanceof ControllerReceiver r) return r;
        }
        return null;
    }

    /** Transmit one input report from {@code player} towards the computer's receiver. */
    public static Result send(ServerPlayer player, UUID controllerId, ControllerState state, TerminalBlockEntity tbe) {
        var eye = player.getEyePosition();
        return send(player.getUUID(), player.level().dimension().location().toString(), eye.x, eye.y - 0.4, eye.z,
                player.getDeltaMovement().length() * 20, controllerId, state, tbe);
    }

    /** Transmit a report from a controller at a world position (hand height), held by {@code playerId}. */
    public static Result send(UUID playerId, String dimension, double x, double y, double z, double speedMps,
                              UUID controllerId, ControllerState state, TerminalBlockEntity tbe) {
        ControllerReceiver receiver = receiverOf(tbe);
        if (receiver == null) return Result.NO_RECEIVER;
        RadioMedium medium = RadioMediumHooks.medium();
        if (medium == null) return Result.NO_MEDIUM;
        Transmitter tx = transmitters.computeIfAbsent(controllerId, Transmitter::new);
        tx.pose = Pose.at(dimension, x, y, z);
        tx.speed = speedMps;
        medium.register(tx);
        byte[] payload = new ControllerFrame(controllerId, playerId, state.buttons(), state.lx(), state.ly(),
                state.rx(), state.ry(), state.lt(), state.rt()).encode();
        Channel ch = receiver.channel();
        medium.transmit(tx, Emission.frame(ch, TX_POWER_DBM, medium.nowMicros(), ControllerFrame.airtimeMicros(),
                "CTRL", 1e6, payload));
        return Result.SENT;
    }

    /** The controller stopped transmitting (disconnected, item gone). */
    public static void stop(UUID controllerId) {
        Transmitter tx = transmitters.remove(controllerId);
        RadioMedium medium = RadioMediumHooks.medium();
        if (tx != null && medium != null) medium.unregister(tx);
        lastDelivery.remove(controllerId);
    }

    static void delivered(UUID controllerId, long nowMs, double rssi) {
        lastDelivery.put(controllerId, new Delivery(nowMs, rssi));
    }

    /** Milliseconds since a report from this controller last got through, or -1 if never. */
    public static long sinceDeliveredMs(UUID controllerId, long nowMs) {
        Delivery d = lastDelivery.get(controllerId);
        return d == null ? -1 : nowMs - d.atMs;
    }

    public static void status(ServerPlayer player, UUID controllerId, int slot, String message) {
        PacketDistributor.sendToPlayer(player, new ControllerPackets.Status(controllerId, slot, message));
    }

    /** A controller as a transmit-only endpoint (a small internal antenna, slightly below isotropic). */
    private static final class Transmitter implements RadioEndpoint {
        final UUID id;
        volatile Pose pose = Pose.at("minecraft:overworld", 0, -10_000, 0);
        volatile double speed;
        static final AntennaPattern PCB = new AntennaPattern() {
            @Override public double gainDbi(double lx, double ly, double lz) { return -2; }
            @Override public double[] polarization(double lx, double ly, double lz) { return new double[] {1, 0, 0}; }
            @Override public double peakGainDbi() { return -2; }
        };

        Transmitter(UUID id) {
            this.id = new UUID(id.getMostSignificantBits() ^ 0x5A5A5A5AL, id.getLeastSignificantBits());
        }

        @Override public UUID id() { return id; }
        @Override public Pose pose() { return pose; }
        @Override public AntennaPattern antenna() { return PCB; }
        @Override public Channel tunedChannel() { return null; }
        @Override public double maxTxPowerDbm() { return TX_POWER_DBM; }
        @Override public double speedMps() { return speed; }
        @Override public void onReceive(Reception reception) {}
    }
}
//?}
