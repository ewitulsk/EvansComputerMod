package com.example.evanscomputermod.radio.controller;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import com.example.evanscomputermod.api.module.IComputerModule;
import com.example.evanscomputermod.api.module.IModuleHost;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.controller.ControllerState;
import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.phys.Vec3;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentLinkedQueue;

/**
 * Controller Receiver: a small 2.4 GHz receiver in a module bay. Wireless
 * Controllers paired with this computer transmit their input reports on its
 * channel; only reports that survive the radio link (distance, walls, other
 * 2.4 GHz traffic such as Wi-Fi on an overlapping channel) reach the computer.
 * Its antenna is the bay, so where the computer stands matters.
 */
public final class ControllerReceiverModule extends AnnotatedPeripheral implements IComputerModule, ControllerReceiver {
    public static final String TYPE = "controller_receiver";
    /** Narrowband channel width of the controller link. */
    public static final double BANDWIDTH_HZ = 2e6;

    private final IModuleHost host;
    private final UUID id = UUID.randomUUID();
    private final ConcurrentLinkedQueue<Reception> inbox = new ConcurrentLinkedQueue<>();
    private final Endpoint endpoint = new Endpoint();
    private volatile int wifiChannel;
    private volatile Pose pose;
    private RadioMedium registeredWith;
    private long received, lastRssiReport;
    private volatile double lastRssi = Double.NaN;

    public ControllerReceiverModule(IModuleHost host, CompoundTag saved) {
        this.host = host;
        int ch = saved.getInt("channel");
        this.wifiChannel = ch >= 1 && ch <= 13 ? ch : 6;
        updatePose();
    }

    @Override
    public String getType() {
        return TYPE;
    }

    // ---------------------------------------------------------- ControllerReceiver

    @Override
    public RadioEndpoint endpoint() {
        return endpoint;
    }

    @Override
    public Channel channel() {
        return channelFor(wifiChannel);
    }

    public static Channel channelFor(int wifiChannel) {
        return new Channel(Channel.wifi24(wifiChannel).centerHz(), BANDWIDTH_HZ);
    }

    // ---------------------------------------------------------- module lifecycle

    @Override
    public void onLoad() {
        updatePose();
    }

    @Override
    public void onUnload() {
        if (registeredWith != null) registeredWith.unregister(endpoint);
        registeredWith = null;
    }

    @Override
    public void tick() {
        RadioMedium medium = RadioMediumHooks.medium();
        if (medium != registeredWith) {
            if (registeredWith != null) registeredWith.unregister(endpoint);
            if (medium != null) medium.register(endpoint);
            registeredWith = medium;
        }
        Pose before = pose;
        updatePose();
        if (medium != null && before != null && pose.movedBeyond(before, 0.5, Math.toRadians(2))) medium.invalidate(endpoint);
        drain();
    }

    private void updatePose() {
        var level = host.getLevel();
        if (level == null) return;
        Vec3 p = Vec3.atCenterOf(host.getPos());
        p = com.example.evanscomputermod.sensor.SensorSable.toWorld(level, p);
        pose = Pose.at(level.dimension().location().toString(), p.x, p.y, p.z);
    }

    /** Apply the reports heard since the last tick to the computer's controllers (server thread). */
    private void drain() {
        if (!(host.getLevel().getBlockEntity(host.getPos()) instanceof TerminalBlockEntity tbe)) {
            inbox.clear();
            return;
        }
        long now = System.currentTimeMillis();
        Reception r;
        while ((r = inbox.poll()) != null) {
            ControllerFrame f = ControllerFrame.decode(r.payload());
            if (f == null) continue;
            received++;
            lastRssi = r.rssiDbm();
            ControllerRadio.delivered(f.controllerId(), now, r.rssiDbm());
            var hub = tbe.getControllers();
            int before = hub.playerOf(f.controllerId());
            int slot = hub.update(f.controllerId(), f.playerId(),
                    new ControllerState(f.buttons(), f.lx(), f.ly(), f.rx(), f.ry(), f.lt(), f.rt()), now);
            ServerPlayer player = host.getLevel().getServer().getPlayerList().getPlayer(f.playerId());
            if (player == null) continue;
            if (slot == 0) {
                ControllerRadio.status(player, f.controllerId(), 0, "Computer already has "
                        + com.example.evanscomputermod.controller.WirelessControllerHub.MAX_CONTROLLERS + " controllers");
            } else if (slot != before || now - lastRssiReport > 2000) {
                lastRssiReport = now;
                ControllerRadio.status(player, f.controllerId(), slot, String.format("Connected (%.0f dBm)", r.rssiDbm()));
            }
        }
    }

    // ---------------------------------------------------------- peripheral API

    @PeripheralMethod(description = "The 2.4 GHz channel (1-13) controllers transmit on", mainThread = false)
    public int get_channel() {
        return wifiChannel;
    }

    @PeripheralMethod(description = "Move to another 2.4 GHz channel (1-13); paired controllers follow")
    public void set_channel(int channel) throws PeripheralException {
        if (channel < 1 || channel > 13) throw new PeripheralException("channel must be 1-13, got " + channel);
        wifiChannel = channel;
        if (registeredWith != null) registeredWith.invalidate(endpoint);
        host.markDirty();
    }

    @PeripheralMethod(description = "{channel, frequency_hz, received, last_rssi_dbm}", mainThread = false)
    public Map<String, Object> stats() {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("channel", wifiChannel);
        m.put("frequency_hz", channel().centerHz());
        m.put("received", received);
        m.put("last_rssi_dbm", Double.isNaN(lastRssi) ? null : lastRssi);
        return m;
    }

    @Override
    public void saveState(CompoundTag tag) {
        tag.putInt("channel", wifiChannel);
    }

    /** The receiver as the radio medium sees it. */
    private final class Endpoint implements RadioEndpoint {
        @Override public UUID id() { return id; }
        @Override public Pose pose() { return pose; }
        @Override public AntennaPattern antenna() { return AntennaPattern.VERTICAL_DIPOLE; }
        @Override public Channel tunedChannel() { return pose == null ? null : channel(); }
        @Override public double maxTxPowerDbm() { return 0; }
        @Override public double sensitivityDbm() { return -92; }
        @Override public void onReceive(Reception reception) {
            if (inbox.size() < 256) inbox.offer(reception);
        }
    }
}
//?}
