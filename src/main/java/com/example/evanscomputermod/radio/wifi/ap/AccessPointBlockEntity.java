package com.example.evanscomputermod.radio.wifi.ap;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.InternetGatewayBlock;
import com.example.evanscomputermod.block.NetworkCableBlock;
import com.example.evanscomputermod.computer.CableNetworkManager;
import com.example.evanscomputermod.computer.NetworkHub;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.medium.BasicRadioMedium;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import com.example.evanscomputermod.radio.wifi80211.MacAddress;
import com.example.evanscomputermod.radio.wifi80211.Security;
import com.example.evanscomputermod.radio.wifi80211.ap.AccessPointCore;
import com.example.evanscomputermod.radio.wifi80211.ap.ApConfig;
import com.example.evanscomputermod.radio.wifi80211.ap.ApOutput;
import com.example.evanscomputermod.radio.wifi80211.ap.ClientStatus;
import com.example.evanscomputermod.sensor.SensorSable;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.LongArrayTag;
import net.minecraft.nbt.Tag;
import net.minecraft.network.chat.Component;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientGamePacketListener;
import net.minecraft.network.protocol.game.ClientboundBlockEntityDataPacket;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.Vec3;
import org.jetbrains.annotations.Nullable;

import java.security.SecureRandom;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * The Access Point: hostapd in a block. A promiscuous bridge port on the
 * cable segment it touches (MAC derived from its UUID) and an 802.11 radio
 * on the shared medium at the block's world pose (projected through Sable
 * when it rides a ship), joined by {@link AccessPointCore} which runs on the
 * server thread every tick. Cable frames arrive on the sender's thread and
 * radio frames on the medium's; both are queued and drained here.
 *
 * <p>The WPA2 passphrase is server-only: it is saved with the block's NBT
 * but {@link #getUpdateTag} (the block-entity sync packet) never contains it.
 */
public class AccessPointBlockEntity extends BlockEntity {
    /** Interface indices used to derive the AP's MACs from its UUID (computers use 0..n). */
    public static final int PORT_MAC_INDEX = 0xA0, BSSID_INDEX = 0xA1;
    public static final int WIRED_QUEUE_LIMIT = 1024;
    /** Checked in tests: never present in the sync tag. */
    public static final String PASSPHRASE_KEY = "Passphrase";

    /** Running APs on this server (server thread), for auto channel selection. */
    private static final Set<AccessPointBlockEntity> LIVE = Collections.newSetFromMap(new java.util.WeakHashMap<>());

    private UUID apId = UUID.randomUUID();
    private @Nullable UUID owner;
    private @Nullable String ownerName;
    private @Nullable ApSettings settings;
    private String passphrase = "";

    // ---- server runtime
    private @Nullable AccessPointCore core;
    private @Nullable WifiAirLink link;
    private final ConcurrentLinkedQueue<byte[]> wiredIn = new ConcurrentLinkedQueue<>();
    private final AtomicInteger wiredInSize = new AtomicInteger();
    private final List<byte[]> radioOut = new ArrayList<>();
    private @Nullable NetworkHub bridgedHub;
    private @Nullable CableNetworkManager cabledManager;
    private @Nullable BlockPos cableExit;
    private boolean cableDirty = true;
    private boolean needsRebuild = true;
    private int channelInUse;
    private int poseTimer;
    private long wiredFramesIn, wiredFramesOut, wiredDropped;
    private @Nullable String lastMessage;

    public AccessPointBlockEntity(BlockPos pos, BlockState state) {
        super(AccessPointContent.ACCESS_POINT_BE.get(), pos, state);
    }

    // ------------------------------------------------------------ identity

    public UUID apId() {
        return apId;
    }

    public byte[] portMac() {
        return NetworkHub.deriveMac(apId, PORT_MAC_INDEX);
    }

    public MacAddress bssid() {
        return MacAddress.of(NetworkHub.deriveMac(apId, BSSID_INDEX));
    }

    public ApSettings settings() {
        if (settings == null) settings = ApSettings.defaults(bssid());
        return settings;
    }

    public boolean hasPassphrase() {
        return !passphrase.isEmpty();
    }

    public @Nullable UUID owner() {
        return owner;
    }

    public @Nullable String ownerName() {
        return ownerName;
    }

    public int channelInUse() {
        return channelInUse;
    }

    public @Nullable AccessPointCore core() {
        return core;
    }

    public @Nullable WifiAirLink link() {
        return link;
    }

    /** True while the AP's port is cabled into a segment. */
    public boolean cabled() {
        CableNetworkManager m = CableNetworkManager.getInstance();
        return m != null && m.networkOf(portMac()) != null;
    }

    // ------------------------------------------------------------ ownership / config

    /** Records the placer as owner (only if unowned). */
    public void claim(Player p) {
        if (owner != null) return;
        owner = p.getUUID();
        ownerName = p.getGameProfile().getName();
        setChanged();
    }

    /** The owner, an operator or a creative-mode player may open and change the settings. */
    public boolean canConfigure(Player p) {
        return owner == null || owner.equals(p.getUUID()) || p.hasPermissions(2) || p.isCreative();
    }

    /**
     * Applies new settings. {@code newPassphrase} empty keeps the stored one (the
     * GUI never sees it). Returns an error message, or null when applied; the
     * radio restarts with the new configuration on the next tick.
     */
    public @Nullable String applySettings(ApSettings s, String newPassphrase) {
        String effective = newPassphrase == null || newPassphrase.isEmpty() ? passphrase : newPassphrase;
        String err = s.validate(effective);
        if (err != null) {
            lastMessage = "Not applied: " + err;
            return err;
        }
        settings = s;
        if (newPassphrase != null && !newPassphrase.isEmpty()) passphrase = newPassphrase;
        needsRebuild = true;
        lastMessage = "Settings applied";
        changedAndSync();
        return null;
    }

    /** Sneak + wrench: factory settings, passphrase erased, and whoever reset it owns it. */
    public void factoryReset(@Nullable Player by) {
        settings = ApSettings.defaults(bssid());
        passphrase = "";
        owner = by == null ? null : by.getUUID();
        ownerName = by == null ? null : by.getGameProfile().getName();
        needsRebuild = true;
        lastMessage = "Factory reset";
        changedAndSync();
        if (by != null) by.displayClientMessage(Component.translatable("message.evanscomputermod.access_point.reset",
                settings.ssid()), true);
    }

    /** Kicks a client off (status page). */
    public boolean deauthenticate(MacAddress mac) {
        if (core == null) return false;
        boolean ok = core.deauthenticate(mac, 2);
        flushRadio();
        return ok;
    }

    private void changedAndSync() {
        setChanged();
        if (level != null && !level.isClientSide()) level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), Block.UPDATE_CLIENTS);
    }

    public void markCableDirty() {
        cableDirty = true;
    }

    // ------------------------------------------------------------ server tick

    public static void serverTick(Level level, BlockPos pos, BlockState state, AccessPointBlockEntity be) {
        if (level instanceof ServerLevel sl) be.tickServer(sl);
    }

    private void tickServer(ServerLevel level) {
        LIVE.add(this);
        RadioMedium medium = RadioMediumHooks.medium();
        if (link == null) {
            link = new WifiAirLink(apId, ApAntenna.INSTANCE, ApSettings.MAX_TX_POWER_DBM, 6, settings().txPowerDbm());
            poseTimer = 0;
        }
        if (poseTimer-- <= 0) {
            link.setPose(worldPose(level));
            poseTimer = 10;
        }
        link.attach(medium);
        attachBridge();
        if (cableDirty || CableNetworkManager.getInstance() != cabledManager) {
            cableDirty = false;
            updateCable(level);
        }
        if (needsRebuild && medium != null) rebuildCore(medium);
        if (core == null || medium == null) {
            wiredIn.clear();
            wiredInSize.set(0);
            return;
        }
        long nowMs = RadioMediumHooks.clockMicros() / 1000;
        AccessPointCore c = core;
        byte[] eth;
        while ((eth = wiredIn.poll()) != null) {
            wiredInSize.decrementAndGet();
            wiredFramesIn++;
            c.onWiredFrame(eth);
        }
        flushRadio();
        link.drain((frame, meta) -> c.onReceive(frame, meta, nowMs));
        flushRadio();
        c.tick(nowMs);
        flushRadio();
        boolean active = cabled();
        if (getBlockState().getValue(AccessPointBlock.ACTIVE) != active)
            level.setBlock(worldPosition, getBlockState().setValue(AccessPointBlock.ACTIVE, active), Block.UPDATE_CLIENTS);
    }

    private Pose worldPose(Level level) {
        Vec3 p = SensorSable.toWorld(level, Vec3.atCenterOf(worldPosition).add(0, 0.3, 0));
        return Pose.at(level.dimension().location().toString(), p.x, p.y, p.z);
    }

    private void flushRadio() {
        if (radioOut.isEmpty()) return;
        List<byte[]> frames = new ArrayList<>(radioOut);
        radioOut.clear();
        if (link != null) for (byte[] f : frames) link.send(f);
    }

    private void attachBridge() {
        NetworkHub hub = NetworkHub.getInstance();
        if (hub == bridgedHub) return;
        if (bridgedHub != null) bridgedHub.unregisterBridgePort(portMac());
        bridgedHub = hub;
        if (hub != null) hub.registerBridgePort(portMac(), frame -> {
            if (wiredInSize.incrementAndGet() > WIRED_QUEUE_LIMIT) {
                wiredInSize.decrementAndGet();
                wiredDropped++;
                return;
            }
            wiredIn.add(frame.clone());
        });
    }

    /** Joins the segment of the first adjacent cable (below first, then above, then the sides). */
    private void updateCable(ServerLevel level) {
        CableNetworkManager mgr = CableNetworkManager.getInstance();
        BlockPos exit = null;
        for (Direction d : Direction.values()) {
            BlockPos q = worldPosition.relative(d);
            Block b = level.getBlockState(q).getBlock();
            if (b instanceof NetworkCableBlock || b instanceof InternetGatewayBlock) {
                exit = q;
                break;
            }
        }
        if (mgr == cabledManager && java.util.Objects.equals(exit, cableExit)) return;
        byte[][] macs = {portMac()};
        if (cabledManager != null && cableExit != null && cabledManager == mgr) mgr.unregisterTerminal(macs);
        cabledManager = mgr;
        cableExit = exit;
        if (mgr != null && exit != null) mgr.registerTerminal(worldPosition, level.dimension(), macs, new BlockPos[] {exit});
    }

    private void rebuildCore(RadioMedium medium) {
        needsRebuild = false;
        if (core != null) {
            core.shutdown();
            flushRadio();
            core = null;
        }
        ApSettings s = settings();
        int ch = s.channelSetting() == 0 ? autoChannel(medium) : s.channelSetting();
        channelInUse = ch;
        link.setChannel(ch);
        link.setTxPowerDbm(s.txPowerDbm());
        try {
            ApConfig cfg = s.toConfig(bssid(), ch, passphrase);
            core = new AccessPointCore(cfg, new Output(), new SecureRandom());
        } catch (IllegalArgumentException e) {
            lastMessage = "Radio off: " + e.getMessage();
            EvansComputerMod.LOGGER.warn("Access point at {}: bad configuration: {}", worldPosition, e.getMessage());
        }
    }

    /**
     * Least busy of 1/6/11: other running APs' received power (path gain from the
     * medium plus their power and peak antenna gains, weighted by channel overlap)
     * plus whatever is on the air right now.
     */
    private int autoChannel(RadioMedium medium) {
        Map<Integer, Double> mw = new HashMap<>();
        for (int c : ApSettings.AUTO_CHANNELS) {
            var ch = ApRadioPlan.channel(c);
            double sum = BasicRadioMedium.dbmToMw(medium.channelPowerDbm(link, ch));
            for (AccessPointBlockEntity other : LIVE) {
                if (other == this || other.isRemoved() || other.link == null || other.link.pose() == null || other.core == null) continue;
                var och = ApRadioPlan.channel(other.channelInUse);
                double w = BasicRadioMedium.overlapFraction(och, ch);
                if (w <= 0) continue;
                double g = medium.pathGainDb(link, other.link, och.centerHz());
                if (Double.isNaN(g) || Double.isInfinite(g)) continue;
                sum += w * BasicRadioMedium.dbmToMw(other.link.txPowerDbm() + 2 * ApAntenna.PEAK_DBI + g);
            }
            mw.put(c, sum <= 0 ? Double.NEGATIVE_INFINITY : BasicRadioMedium.mwToDbm(sum));
        }
        return ApSettings.pickAutoChannel(mw);
    }

    /** Where the core's output goes: the medium (queued, sent after each core call) and the cable. */
    private final class Output implements ApOutput {
        @Override
        public void transmitRadio(byte[] frame80211) {
            radioOut.add(frame80211);
        }

        @Override
        public void transmitWired(byte[] ethernetFrame, MacAddress srcMac) {
            NetworkHub hub = NetworkHub.getInstance();
            if (hub == null) return;
            wiredFramesOut++;
            hub.transmitFromPort(portMac(), ethernetFrame);
        }

        @Override
        public void clientRemoved(MacAddress mac) {
            NetworkHub hub = NetworkHub.getInstance();
            if (hub != null) hub.forgetBridged(mac.bytes());
        }
    }

    // ------------------------------------------------------------ teardown

    @Override
    public void setRemoved() {
        super.setRemoved();
        if (level != null && !level.isClientSide()) teardown();
    }

    private void teardown() {
        LIVE.remove(this);
        if (core != null) {
            for (ClientStatus c : core.clients()) {
                NetworkHub hub = NetworkHub.getInstance();
                if (hub != null) hub.forgetBridged(c.mac().bytes());
            }
            core.shutdown();
            flushRadio();
            core = null;
        }
        needsRebuild = true;
        if (link != null) link.attach(null);
        if (bridgedHub != null && bridgedHub == NetworkHub.getInstance()) bridgedHub.unregisterBridgePort(portMac());
        bridgedHub = null;
        CableNetworkManager mgr = CableNetworkManager.getInstance();
        if (cableExit != null && mgr != null && mgr == cabledManager) mgr.unregisterTerminal(new byte[][] {portMac()});
        cabledManager = null;
        cableExit = null;
        cableDirty = true;
        wiredIn.clear();
        wiredInSize.set(0);
    }

    static void clearLive() {
        LIVE.clear();
    }

    // ------------------------------------------------------------ status for the GUI

    /** What the settings / status screen shows. Never includes the passphrase. */
    public ApPackets.ApView view(@Nullable String message) {
        List<ApPackets.ClientRow> rows = new ArrayList<>();
        List<String> events = List.of();
        if (core != null) {
            for (ClientStatus c : core.clients()) {
                rows.add(new ApPackets.ClientRow(c.mac().toString(), c.lastRssiDbm(), c.lastRateKbps(), c.state().name(),
                        c.handshake().name(), c.lastError() == null ? "" : c.lastError()));
            }
            List<String> all = core.recentEvents();
            events = all.subList(Math.max(0, all.size() - 6), all.size());
        }
        String msg = message != null ? message : lastMessage == null ? "" : lastMessage;
        return new ApPackets.ApView(worldPosition, settings(), hasPassphrase(), bssid().toString(),
                MacAddress.of(portMac()).toString(), channelInUse, cabled(), core != null,
                ownerName == null ? "" : ownerName, rows, List.copyOf(events), msg,
                link == null ? 0 : link.txFrames(), wiredFramesIn, wiredFramesOut);
    }

    // ------------------------------------------------------------ NBT

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        writePublic(tag);
        if (!passphrase.isEmpty()) tag.putString(PASSPHRASE_KEY, passphrase);
    }

    /** Everything a client may know. */
    private void writePublic(CompoundTag tag) {
        tag.putUUID("ApId", apId);
        if (owner != null) tag.putUUID("Owner", owner);
        if (ownerName != null) tag.putString("OwnerName", ownerName);
        ApSettings s = settings();
        tag.putString("Ssid", s.ssid());
        tag.putBoolean("Hidden", s.hidden());
        tag.putString("Security", s.security().name());
        tag.putInt("Channel", s.channelSetting());
        tag.putInt("TxPower", s.txPowerDbm());
        tag.putBoolean("Isolation", s.clientIsolation());
        tag.putString("FilterMode", s.filterMode().name());
        tag.put("FilterMacs", new LongArrayTag(s.filterMacs().stream().mapToLong(MacAddress::value).toArray()));
        tag.putBoolean("HasPassphrase", hasPassphrase());
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        if (tag.hasUUID("ApId")) apId = tag.getUUID("ApId");
        owner = tag.hasUUID("Owner") ? tag.getUUID("Owner") : null;
        ownerName = tag.contains("OwnerName") ? tag.getString("OwnerName") : null;
        if (tag.contains("Ssid", Tag.TAG_STRING)) {
            List<MacAddress> macs = new ArrayList<>();
            for (long v : tag.getLongArray("FilterMacs")) macs.add(new MacAddress(v & 0xFFFF_FFFF_FFFFL));
            settings = new ApSettings(tag.getString("Ssid"), tag.getBoolean("Hidden"), enumOr(Security.class, tag.getString("Security"), Security.OPEN),
                    tag.getInt("Channel"), tag.contains("TxPower") ? tag.getInt("TxPower") : ApSettings.MAX_TX_POWER_DBM,
                    tag.getBoolean("Isolation"), enumOr(ApConfig.MacFilterMode.class, tag.getString("FilterMode"), ApConfig.MacFilterMode.OFF),
                    new ArrayList<>(new LinkedHashSet<>(macs)));
            if (settings.validate(tag.getString(PASSPHRASE_KEY)) != null && level != null && !level.isClientSide())
                settings = ApSettings.defaults(bssid());
        }
        if (tag.contains(PASSPHRASE_KEY)) passphrase = tag.getString(PASSPHRASE_KEY);
        needsRebuild = true;
    }

    private static <E extends Enum<E>> E enumOr(Class<E> type, String name, E fallback) {
        try {
            return Enum.valueOf(type, name);
        } catch (IllegalArgumentException e) {
            return fallback;
        }
    }

    /** The block-entity sync packet's tag: settings and identity, never the passphrase. */
    @Override
    public CompoundTag getUpdateTag(HolderLookup.Provider registries) {
        CompoundTag tag = new CompoundTag();
        writePublic(tag);
        return tag;
    }

    @Override
    public Packet<ClientGamePacketListener> getUpdatePacket() {
        return ClientboundBlockEntityDataPacket.create(this);
    }
}
//?}
