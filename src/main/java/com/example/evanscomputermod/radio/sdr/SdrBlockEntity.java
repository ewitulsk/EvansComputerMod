package com.example.evanscomputermod.radio.sdr;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.RadioConfig;
import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.Vec3;

import java.util.UUID;

/**
 * SDR block state: the {@link SdrRadio} (tuning, rx/tx), its endpoint on the
 * medium and the peripheral computers attach to. With nothing attached the
 * SDR uses its built-in whip (a vertical dipole); with an amplifier, coax or
 * feed point attached it transmits and receives through that chain's antenna
 * ({@link com.example.evanscomputermod.radio.amp.ExciterLink}).
 */
public class SdrBlockEntity extends BlockEntity {

    private final UUID id = UUID.randomUUID();
    private final SdrTier tier;
    private final Endpoint endpoint = new Endpoint();
    private final SdrRadio radio;
    private final SdrPeripheral peripheral;
    private volatile Pose pose;
    private RadioMedium registeredWith;
    /** The transmit chain (amplifier, coax, antenna) attached to this SDR, if any. */
    private final com.example.evanscomputermod.radio.amp.ExciterLink link = new com.example.evanscomputermod.radio.amp.ExciterLink(id);

    public SdrBlockEntity(BlockPos pos, BlockState state) {
        super(RadioSdrContent.SDR_BE.get(), pos, state);
        this.tier = state.getBlock() instanceof SdrBlock b ? b.tier() : SdrTier.BASIC;
        this.radio = new SdrRadio(tier, endpoint, RadioMediumHooks::clockMicros, () -> rateCap(tier),
                this::event, pos.asLong());
        this.peripheral = new SdrPeripheral(radio, RadioMediumHooks::medium, this::setChanged);
        this.radio.setTxGate(e -> !net.neoforged.neoforge.common.NeoForge.EVENT_BUS.post(
                new com.example.evanscomputermod.radio.api.event.RadioTransmitEvent(id, pose, e.channel(), e.powerDbm(), "sdr")).isCanceled());
        this.radio.setTxChain(link::transmit);
    }

    /** The SDR's transmit chain (amplifier / feedline / antenna). */
    public com.example.evanscomputermod.radio.amp.ExciterLink link() {
        return link;
    }

    public SdrTier tier() {
        return tier;
    }

    public SdrPeripheral getPeripheral() {
        return peripheral;
    }

    public RadioEndpoint endpoint() {
        return endpoint;
    }

    /** Sample-rate cap from config, lower on Chicory (pure-Java wasm). */
    static int rateCap(SdrTier tier) {
        int cap = switch (tier) {
            case BASIC -> RadioConfig.sdrBasicRate();
            case STANDARD -> RadioConfig.sdrStandardRate();
            case ADVANCED -> RadioConfig.sdrAdvancedRate();
        };
        String provider = com.example.evanscomputermod.api.wasm.WasmRuntimeRegistry.selectedProviderName();
        if (provider == null || provider.toLowerCase().contains("chicory")) cap = Math.min(cap, RadioConfig.sdrChicoryCap());
        return cap;
    }

    private void event(String name) {
        peripheral.event(name);
    }

    public static void serverTick(Level level, BlockPos pos, BlockState state, SdrBlockEntity be) {
        RadioMedium medium = RadioMediumHooks.medium();
        if (medium != be.registeredWith) {
            if (be.registeredWith != null) be.registeredWith.unregister(be.endpoint);
            if (medium != null) medium.register(be.endpoint);
            be.registeredWith = medium;
        }
        boolean chainChanged = be.link.tick((net.minecraft.server.level.ServerLevel) level, pos, be.whipPose());
        be.updatePose();   // movement: the medium notices it itself (one rate-limited policy)
        if (medium != null && chainChanged) medium.invalidate(be.endpoint);
    }

    private void updatePose() {
        if (level == null) return;
        Pose chain = link.pose();
        pose = chain != null ? chain : whipPose();
    }

    private Pose whipPose() {
        // The whip's feed: its base sits on top of the full-cube case (y 16-17.5 px). The model puts the
        // whip on a rear corner, ~0.3 block off-centre; that offset is ignored here.
        Vec3 p = Vec3.atCenterOf(worldPosition).add(0, 0.6, 0);
        p = com.example.evanscomputermod.sensor.SensorSable.toWorld(level, p);
        return Pose.at(level.dimension().location().toString(), p.x, p.y, p.z);
    }

    @Override
    public void setRemoved() {
        super.setRemoved();
        if (registeredWith != null) registeredWith.unregister(endpoint);
        registeredWith = null;
        link.remove();
    }

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        tag.putDouble("Freq", radio.centerHz());
        tag.putInt("Rate", radio.sampleRate());
        tag.putDouble("Gain", radio.gainDb());
        tag.putBoolean("Agc", radio.agc());
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        // Each setting is clamped into what this tier and the server's caps allow now, so a rate
        // saved under a higher cap doesn't throw away the frequency, gain and AGC with it.
        radio.restore(tag.contains("Freq") ? tag.getDouble("Freq") : null, tag.contains("Rate") ? tag.getInt("Rate") : null,
                tag.contains("Gain") ? tag.getDouble("Gain") : null, tag.contains("Agc") ? tag.getBoolean("Agc") : null);
    }

    private final class Endpoint implements RadioEndpoint {
        @Override public UUID id() { return id; }
        @Override public Pose pose() { return pose; }
        @Override public AntennaPattern antenna() {
            AntennaPattern p = link.pattern(radio.centerHz());
            return p != null ? p : AntennaPattern.VERTICAL_DIPOLE;
        }
        @Override public Channel tunedChannel() { return pose == null ? null : radio.channel(); }
        @Override public double maxTxPowerDbm() { return tier.canTransmit ? link.maxTxDbm(tier.maxTxDbm) : -100; }
        @Override public double noiseFigureDb() { return tier == SdrTier.BASIC ? 8 : 5; }
        @Override public double sensitivityDbm() { return -200; }   // SDRs sample everything; nothing is "decoded"
        @Override public boolean listening() { return false; }        // no frame delivery: IQ is synthesised on read
        @Override public void onReceive(Reception reception) {}
    }
}
//?}
