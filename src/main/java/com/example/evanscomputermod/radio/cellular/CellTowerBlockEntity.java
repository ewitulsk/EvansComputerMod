package com.example.evanscomputermod.radio.cellular;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.InternetGatewayBlock;
import com.example.evanscomputermod.block.NetworkCableBlock;
import com.example.evanscomputermod.computer.CableNetworkManager;
import com.example.evanscomputermod.computer.NetworkHub;
import com.example.evanscomputermod.radio.DirectCablePort;
import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import com.example.evanscomputermod.sensor.SensorSable;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
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

import java.util.ArrayList;
import java.util.EnumMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Objects;
import java.util.UUID;

/**
 * The Cell Tower base station: an eNodeB/gNB for one carrier. It finds the
 * LTE and mmWave antennas mounted on the mast above it, runs one
 * {@link TowerLink} per technology that has antennas, and bridges both onto the
 * cable segment of its RJ45 port (underneath or on a side), like the Access
 * Point: a promiscuous bridge port whose frames to and from devices keep the
 * devices' own MACs.
 *
 * <p><b>Mast.</b> Mast blocks ({@code #evanscomputermod:cell_masts}: Lattice
 * Mast, Antenna Rod, iron bars, chains) stacked on top of the base station;
 * antennas on the side of any mast block (or of the base station) and on top
 * of the mast. Each technology's antenna position is its highest antenna, and
 * its pattern the union of its panels' beams, less the feeder loss down the
 * mast for LTE ({@link CellTech#feederDbPerMetre}).
 *
 * <p>Carrier, channels and owner are saved; devices and links are not (devices
 * re-attach by themselves). Carriers and their SIMs live in the
 * {@link CarrierRegistry}.
 */
public class CellTowerBlockEntity extends BlockEntity {
    public static final int PORT_MAC_INDEX = 0xC0;
    public static final int MAX_MAST = 64;

    private UUID towerId = UUID.randomUUID();
    private @Nullable UUID owner;
    private @Nullable String ownerName;
    private @Nullable String carrier;
    private final EnumMap<CellTech, Integer> channels = new EnumMap<>(CellTech.class);

    // ---- server runtime
    private final EnumMap<CellTech, TowerLink> links = new EnumMap<>(CellTech.class);
    private final EnumMap<CellTech, Mounts> mounts = new EnumMap<>(CellTech.class);
    private @Nullable NetworkHub bridgedHub;
    private @Nullable CableNetworkManager cabledManager;
    private @Nullable BlockPos cableExit;
    private boolean cableDirty = true, antennasDirty = true;
    private DirectCablePort direct;
    private int directTimer, scanTimer, poseTimer;
    private @Nullable String lastMessage;

    /** One technology's antennas: where they are and which way they face. */
    public record Mounts(List<BlockPos> positions, List<Direction> facings, BlockPos top, int mastHeight) {
        static final Mounts NONE = new Mounts(List.of(), List.of(), null, 0);

        public int count() {
            return positions.size();
        }
    }

    public CellTowerBlockEntity(BlockPos pos, BlockState state) {
        super(CellularContent.CELL_TOWER_BE.get(), pos, state);
        for (CellTech t : CellTech.values()) channels.put(t, 1);
    }

    // ------------------------------------------------------------ identity and settings

    public UUID towerId() {
        return towerId;
    }

    public byte[] portMac() {
        return NetworkHub.deriveMac(towerId, PORT_MAC_INDEX);
    }

    public @Nullable String carrier() {
        return carrier;
    }

    public int channel(CellTech t) {
        return channels.get(t);
    }

    public @Nullable String ownerName() {
        return ownerName;
    }

    public @Nullable TowerLink link(CellTech t) {
        return links.get(t);
    }

    public Mounts mounts(CellTech t) {
        return mounts.getOrDefault(t, Mounts.NONE);
    }

    public @Nullable String lastMessage() {
        return lastMessage;
    }

    /** True while the bridge port is cabled into a segment. */
    public boolean cabled() {
        CableNetworkManager m = CableNetworkManager.getInstance();
        return m != null && m.networkOf(portMac()) != null;
    }

    /** Placed by {@code p}: they own it, and it starts on their own carrier ("name-mobile") if that name is free. */
    public void claim(Player p) {
        if (owner != null) return;
        owner = p.getUUID();
        ownerName = p.getGameProfile().getName();
        if (carrier == null && level != null && level.getServer() != null) {
            String id = SimCard.normaliseCarrier(ownerName + "-mobile");
            if (id != null && CarrierRegistry.get(level.getServer()).claim(id, p) != null) carrier = id;
        }
        changedAndSync();
    }

    /** The owner, an operator or a creative player may change the tower's settings. */
    public boolean canConfigure(Player p) {
        return owner == null || owner.equals(p.getUUID()) || p.hasPermissions(2) || p.isCreative();
    }

    /**
     * New carrier and channels (the settings screen's Apply). The player must be allowed
     * to run the carrier (it is created, owned by them, if new). Returns null when applied,
     * else why not.
     */
    public @Nullable String applySettings(Player by, String carrierName, int lteChannel, int mmChannel) {
        String id = SimCard.normaliseCarrier(carrierName);
        if (id == null) return "carrier names are 1-24 letters, digits, '-' or '_'";
        if (!CellTech.LTE.validChannel(lteChannel)) return "LTE channel must be 1-" + CellTech.LTE.channels;
        if (!CellTech.MMWAVE.validChannel(mmChannel)) return "mmWave channel must be 1-" + CellTech.MMWAVE.channels;
        if (level == null || level.getServer() == null) return "not in a world";
        CarrierRegistry reg = CarrierRegistry.get(level.getServer());
        CarrierRegistry.Carrier c = reg.claim(id, by);
        if (c == null) return "carrier " + id + " belongs to " + reg.carrier(id).ownerName;
        carrier = id;
        channels.put(CellTech.LTE, lteChannel);
        channels.put(CellTech.MMWAVE, mmChannel);
        lastMessage = "Settings applied";
        configureLinks();
        changedAndSync();
        return null;
    }

    /** Issue a new SIM of this tower's carrier to {@code by}; returns the SIM, or null with {@link #lastMessage} saying why. */
    public @Nullable SimCard issueSim(Player by, String label) {
        if (level == null || level.getServer() == null) return null;
        CarrierRegistry reg = CarrierRegistry.get(level.getServer());
        CarrierRegistry.Carrier c = reg.carrier(carrier);
        if (c == null) {
            lastMessage = "Set this tower's carrier first";
            return null;
        }
        if (!CarrierRegistry.mayManage(c, by)) {
            lastMessage = "Only " + c.ownerName + " can issue " + c.id + " SIMs";
            return null;
        }
        SimCard s = reg.issue(c, label);
        lastMessage = "Issued " + c.id + " SIM " + s.imsiText();
        return s;
    }

    /** Set a SIM's data limit (bytes, negative = none) for this tower's carrier. */
    public @Nullable String setQuota(Player by, long imsi, long bytes) {
        if (level == null || level.getServer() == null) return "not in a world";
        CarrierRegistry reg = CarrierRegistry.get(level.getServer());
        CarrierRegistry.Carrier c = reg.carrier(carrier);
        if (c == null) return "this tower has no carrier";
        if (!CarrierRegistry.mayManage(c, by)) return "only " + c.ownerName + " can change " + c.id + " SIMs";
        if (!reg.setQuota(c, imsi, bytes)) return "no SIM " + SimCard.formatImsi(imsi) + " on " + c.id;
        lastMessage = bytes < 0 ? "No data limit on " + SimCard.formatImsi(imsi)
                : "Data limit " + bytes + " B on " + SimCard.formatImsi(imsi);
        return null;
    }

    public void markDirty() {
        cableDirty = true;
        antennasDirty = true;
    }

    private void changedAndSync() {
        setChanged();
        if (level != null && !level.isClientSide()) level.sendBlockUpdated(worldPosition, getBlockState(), getBlockState(), Block.UPDATE_CLIENTS);
    }

    // ------------------------------------------------------------ server tick

    public static void serverTick(Level level, BlockPos pos, BlockState state, CellTowerBlockEntity be) {
        if (level instanceof ServerLevel sl) be.tickServer(sl);
    }

    private void tickServer(ServerLevel level) {
        if (links.isEmpty()) {
            CarrierRegistry reg = CarrierRegistry.get(level.getServer());
            for (CellTech t : CellTech.values()) {
                links.put(t, new TowerLink(linkId(towerId, t), t, RadioMediumHooks::medium, reg, this::toCable));
                mounts.put(t, Mounts.NONE);
            }
            configureLinks();
        }
        attachBridge();
        if (cableDirty || CableNetworkManager.getInstance() != cabledManager || directTimer-- <= 0) {
            cableDirty = false;
            directTimer = 40;
            updateCable(level);
            if (direct == null) direct = new DirectCablePort(portMac());
            direct.update(level, worldPosition);
        }
        if (antennasDirty || scanTimer-- <= 0) {
            antennasDirty = false;
            scanTimer = 20;
            scanAntennas(level, false);
        }
        if (poseTimer-- <= 0) {
            poseTimer = 10;
            scanAntennas(level, true);   // follow a Sable ship's transform
        }
        boolean onAir = false;
        for (TowerLink l : links.values()) {
            l.tick();
            onAir |= l.onAir();
        }
        boolean active = onAir && cabled();
        if (getBlockState().getValue(CellTowerBlock.ACTIVE) != active)
            level.setBlock(worldPosition, getBlockState().setValue(CellTowerBlock.ACTIVE, active), Block.UPDATE_CLIENTS);
    }

    /** Each technology's radio has its own id (endpoints and cell keys must differ). */
    static UUID linkId(UUID tower, CellTech t) {
        long salt = 0x9E3779B97F4A7C15L * (t.ordinal() + 1);
        return t.ordinal() == 0 ? tower : new UUID(tower.getMostSignificantBits() ^ salt, tower.getLeastSignificantBits() ^ salt);
    }

    private void configureLinks() {
        for (var e : links.entrySet()) e.getValue().configure(carrier, channels.get(e.getKey()));
    }

    private void toCable(byte[] eth) {
        NetworkHub hub = NetworkHub.getInstance();
        if (hub != null) hub.transmitFromPort(portMac(), eth);
    }

    private void attachBridge() {
        NetworkHub hub = NetworkHub.getInstance();
        if (hub == bridgedHub) return;
        if (bridgedHub != null) bridgedHub.unregisterBridgePort(portMac());
        bridgedHub = hub;
        if (hub != null) hub.registerBridgePort(portMac(), frame -> {
            for (TowerLink l : links.values()) l.fromCable(frame);
        });
    }

    /** Joins the segment of a cable on one of its RJ45 ports (underneath first, then the sides). */
    private void updateCable(ServerLevel level) {
        CableNetworkManager mgr = CableNetworkManager.getInstance();
        BlockPos exit = null;
        BlockState self = getBlockState();
        for (Direction d : Direction.values()) {
            if (!(self.getBlock() instanceof CellTowerBlock b) || !b.acceptsCable(self, d)) continue;
            BlockPos q = worldPosition.relative(d);
            Block nb = level.getBlockState(q).getBlock();
            if (nb instanceof NetworkCableBlock || nb instanceof InternetGatewayBlock) {
                exit = q;
                break;
            }
        }
        if (mgr == cabledManager && Objects.equals(exit, cableExit)) return;
        byte[][] macs = {portMac()};
        if (cabledManager != null && cableExit != null && cabledManager == mgr) mgr.unregisterTerminal(macs);
        cabledManager = mgr;
        cableExit = exit;
        if (mgr != null && exit != null) mgr.registerTerminal(worldPosition, level.dimension(), macs, new BlockPos[] {exit});
    }

    /** Find the mast and its antennas; update each link's antenna (pattern only when the antennas changed). */
    private void scanAntennas(ServerLevel level, boolean poseOnly) {
        if (!poseOnly) {
            List<BlockPos> mast = new ArrayList<>();
            mast.add(worldPosition);
            BlockPos p = worldPosition.above();
            while (mast.size() <= MAX_MAST && level.getBlockState(p).is(CellularContent.CELL_MASTS)) {
                mast.add(p);
                p = p.above();
            }
            Map<CellTech, List<BlockPos>> found = new EnumMap<>(CellTech.class);
            Map<CellTech, List<Direction>> faces = new EnumMap<>(CellTech.class);
            List<BlockPos> candidates = new ArrayList<>();
            for (BlockPos m : mast) for (Direction d : Direction.Plane.HORIZONTAL) candidates.add(m.relative(d));
            candidates.add(p);   // on top of the mast (or of the base station)
            for (BlockPos q : candidates) {
                BlockState s = level.getBlockState(q);
                if (!(s.getBlock() instanceof CellAntennaBlock a)) continue;
                found.computeIfAbsent(a.tech(), k -> new ArrayList<>()).add(q.immutable());
                faces.computeIfAbsent(a.tech(), k -> new ArrayList<>()).add(s.getValue(CellAntennaBlock.FACING));
            }
            for (CellTech t : CellTech.values()) {
                List<BlockPos> ps = found.getOrDefault(t, List.of());
                BlockPos top = null;
                for (BlockPos q : ps) if (top == null || q.getY() > top.getY()) top = q;
                Mounts m = new Mounts(List.copyOf(ps), List.copyOf(faces.getOrDefault(t, List.of())), top,
                        top == null ? 0 : top.getY() - worldPosition.getY());
                if (!m.equals(mounts.get(t))) {
                    mounts.put(t, m);
                    TowerLink l = links.get(t);
                    if (l != null) l.setAntenna(antennaPose(level, m), pattern(t, m));
                }
            }
            return;
        }
        for (CellTech t : CellTech.values()) {
            TowerLink l = links.get(t);
            Mounts m = mounts.get(t);
            if (l != null && m != null && m.top() != null) l.setAntenna(antennaPose(level, m), l.antenna());
        }
    }

    private static Pose antennaPose(ServerLevel level, Mounts m) {
        if (m.top() == null) return null;
        Vec3 p = SensorSable.toWorld(level, Vec3.atCenterOf(m.top()));
        return Pose.at(level.dimension().location().toString(), p.x, p.y, p.z);
    }

    private static AntennaPattern pattern(CellTech t, Mounts m) {
        List<double[]> dirs = new ArrayList<>();
        for (Direction d : m.facings()) dirs.add(new double[] {d.getStepX(), d.getStepZ()});
        return CellAntennas.tower(CellAntennas.panelFor(t), dirs, m.mastHeight() * t.feederDbPerMetre);
    }

    // ------------------------------------------------------------ status

    /** One line per technology for chat / the Jade-less status message. */
    public List<String> statusLines() {
        List<String> out = new ArrayList<>();
        out.add("Cell tower: carrier " + (carrier == null ? "(none)" : carrier) + ", owner " + (ownerName == null ? "-" : ownerName)
                + (cabled() ? ", cabled" : ", NOT cabled"));
        for (CellTech t : CellTech.values()) {
            Mounts m = mounts(t);
            TowerLink l = links.get(t);
            if (m.count() == 0) {
                out.add("  " + t.displayName + ": no antennas");
                continue;
            }
            out.add(String.format(Locale.ROOT, "  %s: ch %d (%.1f MHz), %d antenna%s at %d m, %s, %d device%s", t.displayName, channel(t),
                    t.downlink(channel(t)).centerHz() / 1e6, m.count(), m.count() == 1 ? "" : "s", m.mastHeight(),
                    l != null && l.onAir() ? "on air" : "off air", l == null ? 0 : l.deviceCount(), l != null && l.deviceCount() == 1 ? "" : "s"));
        }
        return out;
    }

    // ------------------------------------------------------------ teardown

    @Override
    public void setRemoved() {
        super.setRemoved();
        if (level != null && !level.isClientSide()) teardown();
    }

    private void teardown() {
        for (TowerLink l : links.values()) l.stop();
        links.clear();
        mounts.clear();
        if (bridgedHub != null && bridgedHub == NetworkHub.getInstance()) bridgedHub.unregisterBridgePort(portMac());
        bridgedHub = null;
        CableNetworkManager mgr = CableNetworkManager.getInstance();
        if (cableExit != null && mgr != null && mgr == cabledManager) mgr.unregisterTerminal(new byte[][] {portMac()});
        if (direct != null) direct.remove();
        cabledManager = null;
        cableExit = null;
        cableDirty = true;
        antennasDirty = true;
    }

    // ------------------------------------------------------------ NBT

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        writePublic(tag);
    }

    private void writePublic(CompoundTag tag) {
        tag.putUUID("TowerId", towerId);
        if (owner != null) tag.putUUID("Owner", owner);
        if (ownerName != null) tag.putString("OwnerName", ownerName);
        if (carrier != null) tag.putString("Carrier", carrier);
        tag.putInt("LteChannel", channels.get(CellTech.LTE));
        tag.putInt("MmChannel", channels.get(CellTech.MMWAVE));
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        if (tag.hasUUID("TowerId")) towerId = tag.getUUID("TowerId");
        owner = tag.hasUUID("Owner") ? tag.getUUID("Owner") : null;
        ownerName = tag.contains("OwnerName") ? tag.getString("OwnerName") : null;
        carrier = tag.contains("Carrier") ? SimCard.normaliseCarrier(tag.getString("Carrier")) : null;
        int lte = tag.getInt("LteChannel"), mm = tag.getInt("MmChannel");
        channels.put(CellTech.LTE, CellTech.LTE.validChannel(lte) ? lte : 1);
        channels.put(CellTech.MMWAVE, CellTech.MMWAVE.validChannel(mm) ? mm : 1);
        if (!links.isEmpty()) configureLinks();
    }

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
