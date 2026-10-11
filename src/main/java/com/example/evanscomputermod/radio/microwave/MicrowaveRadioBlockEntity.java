package com.example.evanscomputermod.radio.microwave;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.InternetGatewayBlock;
import com.example.evanscomputermod.block.NetworkCableBlock;
import com.example.evanscomputermod.computer.CableNetworkManager;
import com.example.evanscomputermod.computer.NetworkHub;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import com.example.evanscomputermod.radio.microwave.dish.DishBlock;
import com.example.evanscomputermod.radio.microwave.dish.DishBlockEntity;
import com.example.evanscomputermod.sensor.SensorSable;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.Vec3;
import org.jetbrains.annotations.Nullable;

import java.util.UUID;

/**
 * The microwave radio's block entity: owns a {@link MicrowaveLink}, joins the
 * adjacent cable as a bridge port (MAC from its persistent UUID), feeds the
 * adjacent dish (its world pose and diameter become the endpoint's antenna)
 * and samples Minecraft weather for rain fade.
 */
public class MicrowaveRadioBlockEntity extends BlockEntity {
    /** NIC index byte for radio MACs, so they never collide with a computer's eth0-5. */
    static final int MAC_INDEX = 0x6d;

    private UUID id = UUID.randomUUID();
    private MwBand band = MwBand.GHZ_24;
    private int widthMhz = MwBand.GHZ_24.defaultWidthMhz, channel = 0;
    private double txPowerDbm = MicrowaveLink.DEFAULT_TX_DBM;

    private @Nullable MicrowaveLink link;
    private @Nullable MicrowaveRadioPeripheral peripheral;
    private @Nullable NetworkHub bridgedHub;
    private @Nullable CableNetworkManager cabledManager;
    private @Nullable BlockPos cableExit;
    private boolean cableDirty = true;
    private int weatherTimer;
    private com.example.evanscomputermod.radio.DirectCablePort direct;
    private int directTimer;

    public MicrowaveRadioBlockEntity(BlockPos pos, BlockState state) {
        super(MicrowaveContent.RADIO_BE.get(), pos, state);
    }

    public byte[] mac() {
        return NetworkHub.deriveMac(id, MAC_INDEX);
    }

    /** The radio engine (created on first use, with the saved settings). */
    public MicrowaveLink link() {
        if (link == null) {
            link = new MicrowaveLink(id, mac(), RadioMediumHooks::medium, frame -> {
                NetworkHub hub = NetworkHub.getInstance();
                if (hub != null) hub.transmitFromPort(mac(), frame);
            }, System::currentTimeMillis);
            try {
                link.configure(band, widthMhz, channel);
                link.setTxPowerDbm(txPowerDbm);
            } catch (IllegalArgumentException bad) {
                link.configure(MwBand.GHZ_24, MwBand.GHZ_24.defaultWidthMhz, 0);
            }
        }
        return link;
    }

    public MicrowaveRadioPeripheral getPeripheral() {
        if (peripheral == null) peripheral = new MicrowaveRadioPeripheral(link(), this::setChanged);
        return peripheral;
    }

    public void markCableDirty() {
        cableDirty = true;
    }

    /** True while the radio's port is cabled into a segment. */
    public boolean cabled() {
        CableNetworkManager m = CableNetworkManager.getInstance();
        return m != null && m.networkOf(mac()) != null;
    }

    public static void serverTick(Level level, BlockPos pos, BlockState state, MicrowaveRadioBlockEntity be) {
        if (level instanceof ServerLevel sl) be.tickServer(sl);
    }

    private void tickServer(ServerLevel level) {
        MicrowaveLink l = link();
        attachBridge();
        if (cableDirty || CableNetworkManager.getInstance() != cabledManager || directTimer-- <= 0) {
            cableDirty = false;
            directTimer = 40;   // a touching computer's faces are known once it has booted
            updateCable(level);
            if (direct == null) direct = new com.example.evanscomputermod.radio.DirectCablePort(mac());
            direct.update(level, worldPosition);
        }
        DishBlockEntity dish = findDish(level);
        if (dish != null) {
            dish.linkRadio(l);
            l.setDish(dish.worldPose(), dish.size().diameterM);
        } else {
            l.setDish(null, 0);
        }
        if (weatherTimer-- <= 0) {
            weatherTimer = 20;
            l.setRainRate(rainRateAt(level, BlockPos.containing(SensorSable.toWorld(level, Vec3.atCenterOf(worldPosition)))));
        }
        l.tick();
    }

    /** The dish touching this radio (any part of it), or null. */
    public @Nullable DishBlockEntity findDish(Level level) {
        for (Direction d : Direction.values()) {
            BlockPos q = worldPosition.relative(d);
            if (level.getBlockState(q).getBlock() instanceof DishBlock) {
                DishBlockEntity be = DishBlock.controllerEntity(level, q);
                if (be != null) return be;
            }
        }
        return null;
    }

    /**
     * Rain rate at a position, mm/h: Minecraft's rain level times 10 mm/h,
     * rising to 50 with the thunder level, where the biome rains; a third of
     * that where it snows; none in dry biomes.
     */
    public static double rainRateAt(Level level, BlockPos pos) {
        float rain = level.getRainLevel(1f);
        if (rain <= 0) return 0;
        double rate = rain * (net.minecraft.util.Mth.lerp(level.getThunderLevel(1f), 10.0, 50.0));
        Biome.Precipitation p = level.getBiome(pos).value().getPrecipitationAt(pos);
        return switch (p) {
            case RAIN -> rate;
            case SNOW -> rate / 3;
            default -> 0;
        };
    }

    private void attachBridge() {
        NetworkHub hub = NetworkHub.getInstance();
        if (hub == bridgedHub) return;
        if (bridgedHub != null) bridgedHub.unregisterBridgePort(mac());
        bridgedHub = hub;
        MicrowaveLink l = link();
        if (hub != null) hub.registerBridgePort(mac(), l::fromCable);
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
        byte[][] macs = {mac()};
        if (cabledManager != null && cableExit != null && cabledManager == mgr) mgr.unregisterTerminal(macs);
        cabledManager = mgr;
        cableExit = exit;
        if (mgr != null && exit != null) mgr.registerTerminal(worldPosition, level.dimension(), macs, new BlockPos[] {exit});
    }

    @Override
    public void setRemoved() {
        super.setRemoved();
        if (link != null) link.stop();
        if (bridgedHub != null && bridgedHub == NetworkHub.getInstance()) bridgedHub.unregisterBridgePort(mac());
        bridgedHub = null;
        CableNetworkManager mgr = CableNetworkManager.getInstance();
        if (cableExit != null && mgr != null && mgr == cabledManager) mgr.unregisterTerminal(new byte[][] {mac()});
        if (direct != null) direct.remove();
        cabledManager = null;
        cableExit = null;
        cableDirty = true;
    }

    @Override
    public void clearRemoved() {
        super.clearRemoved();
        cableDirty = true;
    }

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        MicrowaveLink l = link;
        tag.putUUID("Id", id);
        tag.putInt("Band", l == null ? band.ghz : l.band().ghz);
        tag.putInt("Width", l == null ? widthMhz : l.widthMhz());
        tag.putInt("Channel", l == null ? channel : l.channelNumber());
        tag.putDouble("TxPower", l == null ? txPowerDbm : l.txPowerDbm());
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        if (tag.hasUUID("Id")) id = tag.getUUID("Id");
        try {
            if (tag.contains("Band")) band = MwBand.of(tag.getInt("Band"));
        } catch (IllegalArgumentException ignored) {
            band = MwBand.GHZ_24;
        }
        if (tag.contains("Width")) widthMhz = tag.getInt("Width");
        if (tag.contains("Channel")) channel = tag.getInt("Channel");
        if (tag.contains("TxPower")) txPowerDbm = Math.max(MicrowaveLink.MIN_TX_DBM, Math.min(MicrowaveLink.MAX_TX_DBM, tag.getDouble("TxPower")));
    }
}
//?}
