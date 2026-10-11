package com.example.evanscomputermod.radio.wifi;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.IComputerModule;
import com.example.evanscomputermod.api.module.IModuleHost;
import com.example.evanscomputermod.api.peripheral.AnnotatedPeripheral;
import com.example.evanscomputermod.api.peripheral.PeripheralException;
import com.example.evanscomputermod.api.peripheral.PeripheralMethod;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.controller.ControllerState;
import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;
import com.example.evanscomputermod.radio.controller.ControllerFrame;
import com.example.evanscomputermod.radio.controller.ControllerRadio;
import com.example.evanscomputermod.radio.controller.ControllerReceiver;
import com.example.evanscomputermod.radio.controller.ControllerReceiverModule;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import com.example.evanscomputermod.radio.wifi.mac.LowMac;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.phys.Vec3;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentLinkedQueue;

/**
 * Wi-Fi module: a SoftMAC 2.4/5 GHz radio in a module bay (up to 20 dBm, a
 * small dipole at the bay). In {@code wifi} mode the computer's kernel drives it
 * as {@code wlan0} through the {@code wifi_*} host functions; the timing-critical
 * MAC (FCS, ACK, retries, CSMA/CA, airtime, RX filter) is {@link LowMac}. In
 * {@code controller} mode it is a 2.4 GHz Wireless Controller receiver instead
 * (narrow 2 MHz channel, like the dedicated Controller Receiver), and the kernel
 * sees no Wi-Fi radio.
 */
public final class WifiModule extends AnnotatedPeripheral implements IComputerModule, WifiRadio, ControllerReceiver {
    public static final String TYPE = "wifi";
    public static final String MODE_WIFI = "wifi";
    public static final String MODE_CONTROLLER = "controller";

    private final IModuleHost host;
    private final UUID id = UUID.randomUUID();
    private final Endpoint endpoint = new Endpoint();
    private final LowMac mac;
    private final ConcurrentLinkedQueue<Reception> controllerInbox = new ConcurrentLinkedQueue<>();
    private volatile String mode;
    private volatile int controllerChannel;
    private volatile Pose pose;
    private volatile boolean live;
    private RadioMedium registeredWith;
    private long controllerReports, lastRssiReport;
    private volatile double lastControllerRssi = Double.NaN;

    public WifiModule(IModuleHost host, CompoundTag saved) {
        this.host = host;
        byte[] m = new byte[6];
        long stored = saved.getLong("mac");
        if (stored == 0) {
            UUID u = UUID.randomUUID();
            stored = u.getMostSignificantBits() ^ u.getLeastSignificantBits();
        }
        for (int i = 0; i < 6; i++) m[i] = (byte) (stored >>> (8 * (5 - i)));
        m[0] = (byte) ((m[0] & 0xfc) | 0x02);       // locally administered, unicast
        int ch = saved.getInt("channel");
        this.mac = new LowMac(endpoint, RadioMediumHooks::medium, m, ch >= 1 ? ch : 1, LowMac.Options.world(stored));
        double p = saved.contains("max_power_dbm") ? saved.getDouble("max_power_dbm") : LowMac.MAX_TX_POWER_DBM;
        this.mac.setMaxPowerDbm(p);
        this.mode = MODE_CONTROLLER.equals(saved.getString("mode")) ? MODE_CONTROLLER : MODE_WIFI;
        int cc = saved.getInt("controller_channel");
        this.controllerChannel = cc >= 1 && cc <= 13 ? cc : 6;
        updatePose();
    }

    @Override
    public String getType() {
        return TYPE;
    }

    // ---------------------------------------------------------- WifiRadio (kernel side)

    @Override
    public boolean wifiActive() {
        return live && MODE_WIFI.equals(mode);
    }

    @Override
    public LowMac mac() {
        return mac;
    }

    // ---------------------------------------------------------- ControllerReceiver

    @Override
    public RadioEndpoint endpoint() {
        return endpoint;
    }

    @Override
    public Channel channel() {
        return ControllerReceiverModule.channelFor(controllerChannel);
    }

    @Override
    public boolean receivingControllers() {
        return live && MODE_CONTROLLER.equals(mode);
    }

    // ---------------------------------------------------------- module lifecycle

    @Override
    public void onLoad() {
        live = true;
        updatePose();
    }

    @Override
    public void onUnload() {
        live = false;
        if (registeredWith != null) registeredWith.unregister(endpoint);
        registeredWith = null;
    }

    @Override
    public void tick() {
        live = host.isAlive();
        RadioMedium medium = RadioMediumHooks.medium();
        if (medium != registeredWith) {
            if (registeredWith != null) registeredWith.unregister(endpoint);
            if (medium != null) medium.register(endpoint);
            registeredWith = medium;
        }
        updatePose();   // the medium notices the move itself (one rate-limited policy)
        if (MODE_CONTROLLER.equals(mode)) {
            drainControllerReports();
        } else {
            controllerInbox.clear();
        }
    }

    private void updatePose() {
        var level = host.getLevel();
        if (level == null) return;
        Vec3 p = Vec3.atCenterOf(host.getPos());
        p = com.example.evanscomputermod.sensor.SensorSable.toWorld(level, p);
        pose = Pose.at(level.dimension().location().toString(), p.x, p.y, p.z);
    }

    /** Apply Wireless Controller reports heard in controller mode (server thread). */
    private void drainControllerReports() {
        if (!(host.getLevel().getBlockEntity(host.getPos()) instanceof TerminalBlockEntity tbe)) {
            controllerInbox.clear();
            return;
        }
        long now = System.currentTimeMillis();
        Reception r;
        while ((r = controllerInbox.poll()) != null) {
            ControllerFrame f = ControllerFrame.decode(r.payload());
            if (f == null) continue;
            controllerReports++;
            lastControllerRssi = r.rssiDbm();
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
                ControllerRadio.status(player, f.controllerId(), slot, String.format("Connected (%.0f dBm, Wi-Fi module)", r.rssiDbm()));
            }
        }
    }

    // ---------------------------------------------------------- peripheral API

    @PeripheralMethod(description = "\"wifi\" (wlan0 for the kernel) or \"controller\" (Wireless Controller receiver)", mainThread = false)
    public String get_mode() {
        return mode;
    }

    @PeripheralMethod(description = "Switch between \"wifi\" and \"controller\" mode")
    public void set_mode(String m) throws PeripheralException {
        if (!MODE_WIFI.equals(m) && !MODE_CONTROLLER.equals(m)) {
            throw new PeripheralException("mode must be \"wifi\" or \"controller\", got " + m);
        }
        if (m.equals(mode)) return;
        mode = m;
        controllerInbox.clear();
        while (mac.poll() != null) { /* drop Wi-Fi frames heard in the old mode */ }
        if (registeredWith != null) registeredWith.invalidate(endpoint);
        host.markDirty();
    }

    @PeripheralMethod(description = "Channel: the Wi-Fi channel in wifi mode (set by the kernel), the controller channel (1-13) in controller mode", mainThread = false)
    public int get_channel() {
        return MODE_CONTROLLER.equals(mode) ? controllerChannel : mac.channelNumber();
    }

    @PeripheralMethod(description = "Controller-mode channel (1-13); paired controllers follow")
    public void set_controller_channel(int channel) throws PeripheralException {
        if (channel < 1 || channel > 13) throw new PeripheralException("channel must be 1-13, got " + channel);
        controllerChannel = channel;
        if (registeredWith != null) registeredWith.invalidate(endpoint);
        host.markDirty();
    }

    @PeripheralMethod(description = "Highest transmit power in dBm (0-20)")
    public void set_max_power(double dbm) throws PeripheralException {
        if (!(dbm >= 0 && dbm <= LowMac.MAX_TX_POWER_DBM)) throw new PeripheralException("power must be 0-20 dBm");
        mac.setMaxPowerDbm(dbm);
        host.markDirty();
    }

    @PeripheralMethod(description = "The radio's MAC address (aa:bb:cc:dd:ee:ff)", mainThread = false)
    public String get_mac() {
        byte[] m = mac.mac();
        return String.format("%02x:%02x:%02x:%02x:%02x:%02x", m[0], m[1], m[2], m[3], m[4], m[5]);
    }

    @PeripheralMethod(description = "{mode, channel, frequency_hz, rx_filter, tx_frames, tx_attempts, tx_failed, rx_frames, rx_dropped, acks_sent, acks_received, cca_deferrals, airtime_us, controller_reports, last_controller_rssi_dbm}", mainThread = false)
    public Map<String, Object> stats() {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("mode", mode);
        m.put("channel", get_channel());
        Channel c = MODE_CONTROLLER.equals(mode) ? channel() : mac.channel();
        m.put("frequency_hz", c.centerHz());
        m.put("rx_filter", switch (mac.mode()) { case LowMac.MODE_MONITOR -> "monitor"; case LowMac.MODE_PROMISC -> "promiscuous"; default -> "normal"; });
        m.put("max_power_dbm", mac.maxPowerDbm());
        m.put("tx_frames", mac.txFrames.get());
        m.put("tx_attempts", mac.txAttempts.get());
        m.put("tx_failed", mac.txFailed.get());
        m.put("rx_frames", mac.rxFrames.get());
        m.put("rx_dropped", mac.rxDropped.get());
        m.put("acks_sent", mac.acksSent.get());
        m.put("acks_received", mac.acksReceived.get());
        m.put("cca_deferrals", mac.ccaDeferrals.get());
        m.put("airtime_us", mac.airtimeUs.get());
        m.put("controller_reports", controllerReports);
        m.put("last_controller_rssi_dbm", Double.isNaN(lastControllerRssi) ? null : lastControllerRssi);
        return m;
    }

    @Override
    public void saveState(CompoundTag tag) {
        byte[] m = mac.mac();
        long v = 0;
        for (byte b : m) v = v << 8 | (b & 0xff);
        tag.putLong("mac", v);
        tag.putString("mode", mode);
        tag.putInt("channel", mac.channelNumber());
        tag.putInt("controller_channel", controllerChannel);
        tag.putDouble("max_power_dbm", mac.maxPowerDbm());
    }

    /** The module's antenna on the medium: Wi-Fi channel or the controller channel by mode. */
    private final class Endpoint implements RadioEndpoint {
        @Override public UUID id() { return id; }
        @Override public Pose pose() { return pose; }
        @Override public AntennaPattern antenna() { return AntennaPattern.VERTICAL_DIPOLE; }
        @Override public Channel tunedChannel() {
            if (pose == null || !live) return null;
            return MODE_CONTROLLER.equals(mode) ? channel() : mac.channel();
        }
        @Override public double maxTxPowerDbm() { return mac.maxPowerDbm(); }
        @Override public double sensitivityDbm() { return -92; }
        @Override public void onReceive(Reception reception) {
            if (MODE_CONTROLLER.equals(mode)) {
                if (controllerInbox.size() < 256) controllerInbox.offer(reception);
            } else {
                mac.onReceive(reception);
            }
        }
    }
}
//?}
