package com.example.evanscomputermod.radio.amp;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.api.event.HazardEvent;
import com.example.evanscomputermod.radio.hazard.HazardActions;
import com.example.evanscomputermod.radio.hazard.RadioHazardContent;
import com.example.evanscomputermod.radio.hazard.ThermalModel;
import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.particles.ParticleTypes;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import org.jetbrains.annotations.Nullable;

import java.util.Locale;
import java.util.UUID;

/** Tuner state: temperature from the mismatch power it absorbs, and who placed it. */
public class TunerBlockEntity extends BlockEntity {
    private double theta;
    private double pendingHeatW;
    private volatile double lastHeatW;
    private volatile double lastSwrIn = 1, lastSwrOut = 1;
    private long lastTxTick = Long.MIN_VALUE;
    @Nullable private UUID owner;

    public TunerBlockEntity(BlockPos pos, BlockState state) {
        super(RadioAmpContent.TUNER_BE.get(), pos, state);
    }

    public double theta() { return theta; }
    public double lastHeatW() { return lastHeatW; }
    @Nullable public UUID owner() { return owner; }

    public void setOwner(@Nullable UUID owner) {
        this.owner = owner;
        setChanged();
    }

    /** Feed point of the chain this tuner last worked in (for the AntennaOverloadEvent), or null. */
    private BlockPos lastFeed;

    void deposit(double heatW, double swrAntenna, boolean matched, @org.jetbrains.annotations.Nullable BlockPos feed) {
        pendingHeatW += heatW;
        if (feed != null) lastFeed = feed;
        lastSwrOut = swrAntenna;
        lastSwrIn = matched ? 1 : swrAntenna;
        if (level != null) lastTxTick = level.getGameTime();
    }

    public static void serverTick(Level level, BlockPos pos, BlockState state, TunerBlockEntity be) {
        be.tick((ServerLevel) level, pos);
    }

    private void tick(ServerLevel level, BlockPos pos) {
        lastHeatW = pendingHeatW;
        theta = ThermalModel.Part.TUNER.step(theta, pendingHeatW / AmpModel.TUNER_RATING_W);
        pendingHeatW = 0;
        long now = level.getGameTime();
        if (theta >= ThermalModel.WARNING && now % 10 == 0) HazardActions.burst(level, pos, ParticleTypes.SMOKE, 3);
        if (theta >= 1 && now % 20 == 0) {
            // Like every other chain failure: an AntennaOverloadEvent first (cancelling it spares the tuner).
            if (HazardActions.equipmentHazards(level, pos) && HazardActions.overload(level, lastFeed != null ? lastFeed : pos,
                    lastHeatW, AmpModel.TUNER_RATING_W, pos, "tuner_mismatch")) {
                HazardActions.destroy(level, pos, owner, HazardEvent.Kind.MELT,
                        String.format(Locale.ROOT, "tuner burnt out absorbing %.0f W of mismatch (SWR %.1f)", lastHeatW, lastSwrOut),
                        new ItemStack(RadioHazardContent.MELTED_SCRAP.get()));
            }
            if (!isRemoved()) theta = 1;
        }
    }

    public String statusLine() {
        boolean tx = level != null && level.getGameTime() - lastTxTick <= 20;
        return String.format(Locale.ROOT, "Antenna Tuner: %s · %.0f °C (rated %.0f W of mismatch)",
                tx ? String.format(Locale.ROOT, "antenna SWR %.1f:1 matched to %.1f:1, absorbing %.0f W", lastSwrOut, lastSwrIn, lastHeatW) : "idle",
                ThermalModel.Part.TUNER.celsius(theta), AmpModel.TUNER_RATING_W);
    }

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        tag.putDouble("Theta", theta);
        if (owner != null) tag.putUUID("Owner", owner);
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        theta = tag.getDouble("Theta");
        owner = tag.hasUUID("Owner") ? tag.getUUID("Owner") : null;
    }
}
//?}
