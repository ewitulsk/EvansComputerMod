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
 * medium and the peripheral computers attach to. Until a coax feed to a built
 * antenna exists, the SDR uses its built-in whip (a vertical dipole).
 */
public class SdrBlockEntity extends BlockEntity {

    private final UUID id = UUID.randomUUID();
    private final SdrTier tier;
    private final Endpoint endpoint = new Endpoint();
    private final SdrRadio radio;
    private final SdrPeripheral peripheral;
    private volatile Pose pose;
    private RadioMedium registeredWith;

    public SdrBlockEntity(BlockPos pos, BlockState state) {
        super(RadioSdrContent.SDR_BE.get(), pos, state);
        this.tier = state.getBlock() instanceof SdrBlock b ? b.tier() : SdrTier.BASIC;
        this.radio = new SdrRadio(tier, endpoint, RadioMediumHooks::clockMicros, () -> rateCap(tier),
                this::event, pos.asLong());
        this.peripheral = new SdrPeripheral(radio, RadioMediumHooks::medium, this::setChanged);
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
        Pose before = be.pose;
        be.updatePose();
        if (medium != null && before != null && be.pose.movedBeyond(before, RadioConfig.sableRecomputeMetres(), RadioConfig.sableRecomputeRadians()))
            medium.invalidate(be.endpoint);
    }

    private void updatePose() {
        if (level == null) return;
        Vec3 p = Vec3.atCenterOf(worldPosition).add(0, 0.6, 0);   // the whip's feed
        p = com.example.evanscomputermod.sensor.SensorSable.toWorld(level, p);
        pose = Pose.at(level.dimension().location().toString(), p.x, p.y, p.z);
    }

    @Override
    public void setRemoved() {
        super.setRemoved();
        if (registeredWith != null) registeredWith.unregister(endpoint);
        registeredWith = null;
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
        try {
            if (tag.contains("Freq")) radio.setFrequency(tag.getDouble("Freq"));
            if (tag.contains("Rate")) radio.setSampleRate(tag.getInt("Rate"));
            if (tag.contains("Gain")) radio.setGain(tag.getDouble("Gain"));
            radio.setAgc(!tag.contains("Agc") || tag.getBoolean("Agc"));
        } catch (IllegalArgumentException ignored) {
            // Settings from a different tier or config: keep defaults.
        }
    }

    private final class Endpoint implements RadioEndpoint {
        @Override public UUID id() { return id; }
        @Override public Pose pose() { return pose; }
        @Override public AntennaPattern antenna() { return AntennaPattern.VERTICAL_DIPOLE; }
        @Override public Channel tunedChannel() { return pose == null ? null : radio.channel(); }
        @Override public double maxTxPowerDbm() { return tier.canTransmit ? tier.maxTxDbm : -100; }
        @Override public double noiseFigureDb() { return tier == SdrTier.BASIC ? 8 : 5; }
        @Override public double sensitivityDbm() { return -200; }   // SDRs sample everything; nothing is "decoded"
        @Override public boolean listening() { return false; }        // no frame delivery: IQ is synthesised on read
        @Override public void onReceive(Reception reception) {}
    }
}
//?}
