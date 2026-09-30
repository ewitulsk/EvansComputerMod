package com.example.evanscomputermod.sensor;

//? if <=1.21.1 {
import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.component.DataComponentMap;
import net.minecraft.core.component.DataComponents;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.chat.Component;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import org.jetbrains.annotations.Nullable;

/**
 * A lidar sensor's block entity. It holds the sensor's name (from an
 * anvil-renamed item); scanning is driven by the Wired Sensor Module it is
 * wired to. Its renderer draws the spinning head.
 */
public class LidarSensorBlockEntity extends BlockEntity {
    /** Ticks without a scan before the head stops spinning. */
    private static final long IDLE_TICKS = 40;

    @Nullable
    private Component customName;
    private long lastScan;

    public LidarSensorBlockEntity(BlockPos pos, BlockState state) {
        super(SensorContent.LIDAR_SENSOR_BE.get(), pos, state);
    }

    /** The name programs see, or null to get an automatic {@code lidar_N}. */
    @Nullable
    public String sensorName() {
        if(customName == null)
            return null;
        String s = customName.getString().trim();
        return s.isEmpty() ? null : s;
    }

    /** A module is scanning with this sensor (server). */
    public void touch(long gameTime) {
        lastScan = gameTime;
        if(level != null && !getBlockState().getValue(LidarSensorBlock.ACTIVE))
            level.setBlock(worldPosition, getBlockState().setValue(LidarSensorBlock.ACTIVE, true), 3);
    }

    public static void serverTick(net.minecraft.world.level.Level level, BlockPos pos, BlockState state, LidarSensorBlockEntity be) {
        if(state.getValue(LidarSensorBlock.ACTIVE) && level.getGameTime() - be.lastScan > IDLE_TICKS)
            level.setBlock(pos, state.setValue(LidarSensorBlock.ACTIVE, false), 3);
    }

    @Override
    protected void applyImplicitComponents(DataComponentInput input) {
        super.applyImplicitComponents(input);
        customName = input.get(DataComponents.CUSTOM_NAME);
    }

    @Override
    protected void collectImplicitComponents(DataComponentMap.Builder builder) {
        super.collectImplicitComponents(builder);
        builder.set(DataComponents.CUSTOM_NAME, customName);
    }

    @Override
    public void removeComponentsFromTag(CompoundTag tag) {
        tag.remove("CustomName");
    }

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        if(customName != null)
            tag.putString("CustomName", Component.Serializer.toJson(customName, registries));
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        customName = tag.contains("CustomName") ? Component.Serializer.fromJson(tag.getString("CustomName"), registries) : null;
    }
}
//?}
