package com.example.evanscomputermod.block;

import com.example.evanscomputermod.entity.MissileEntity;
import com.example.evanscomputermod.missile.IMissileMount;
import com.example.evanscomputermod.missile.MissileTier;
import com.example.evanscomputermod.missile.WarheadType;
import com.example.evanscomputermod.network.LauncherStatePacket;
import net.minecraft.core.BlockPos;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.protocol.Packet;
import net.minecraft.network.protocol.game.ClientGamePacketListener;
import net.minecraft.network.protocol.game.ClientboundBlockEntityDataPacket;
//? if >=26.1 {
import net.minecraft.world.level.storage.ValueInput;
import net.minecraft.world.level.storage.ValueOutput;
//?}
import net.minecraft.server.level.ServerLevel;
import net.minecraft.util.Mth;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.BlockEntityTicker;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockState;
import net.neoforged.neoforge.network.PacketDistributor;

import org.jetbrains.annotations.Nullable;
import java.util.UUID;

/**
 * Manages a single launcher's state: bearing, elevation, loaded missile tier,
 * yield override, GPS target, and redstone arm/fire logic.
 *
 * <p>Implements {@link IMissileMount} so the Python module and any future
 * multi-block silo controller can address it through the same contract.
 */
public class MissileLauncherBlockEntity extends BlockEntity implements IMissileMount {

    // ── Configurable state ────────────────────────────────────────────────

    /** Compass bearing degrees: 0 = North, 90 = East, 180 = South, 270 = West. */
    private float bearing   = 0f;
    /** Elevation degrees above horizontal: 5–85. */
    private float elevation = 45f;
    /** Loaded missile tier, or null if the launcher is empty. */
    @Nullable private MissileTier loadedTier = null;
    /** Warhead yield override; NaN means "use tier default". */
    private float yieldOverride = Float.NaN;
    /** GPS target position, or null if unset. */
    @Nullable private BlockPos targetPos = null;

    // ── Redstone state ────────────────────────────────────────────────────

    private boolean prevRedstonePowered = false;

    // ──────────────────────────────────────────────────────────────────────

    public MissileLauncherBlockEntity(BlockPos pos, BlockState state) {
        super(ModBlockEntities.MISSILE_LAUNCHER_BLOCK_ENTITY.get(), pos, state);
    }

    // ── IMissileMount ─────────────────────────────────────────────────────

    @Override public float getBearing()                       { return bearing; }
    @Override public float getElevation()                     { return elevation; }
    @Override @Nullable public MissileTier getLoadedTier()    { return loadedTier; }
    @Override public boolean isLoaded()                       { return loadedTier != null; }
    @Override public BlockPos getMountPos()                   { return worldPosition; }
    @Override public Level    getMountLevel()                  { return level; }

    @Override
    public void onLaunch() {
        loadedTier = null;
        setChanged();
    }

    // ── Client-side setters (called by LauncherStatePacket handler) ───────

    /** Only called on the client from the network packet handler. */
    public void setClientBearing(float deg)   { this.bearing   = deg; }
    /** Only called on the client from the network packet handler. */
    public void setClientElevation(float deg) { this.elevation = deg; }

    // ── Python-facing setters ─────────────────────────────────────────────

    public void setBearing(float deg) {
        this.bearing = ((deg % 360) + 360) % 360;
        syncToClients();
        setChanged();
    }

    public void setElevation(float deg) {
        this.elevation = Mth.clamp(deg, 5f, 85f);
        syncToClients();
        setChanged();
    }

    /** Load a missile tier into the launcher (creative mode — no item consumed). */
    public void loadMissile(MissileTier tier) {
        this.loadedTier = tier;
        setChanged();
    }

    /** Set the GPS target. Pass null to clear. */
    public void setTarget(@Nullable BlockPos target) {
        this.targetPos = target;
        setChanged();
    }

    @Nullable public BlockPos getTargetPos() { return targetPos; }

    /** Override yield (0.0–1.0). Pass {@link Float#NaN} to restore tier default. */
    public void setYieldOverride(float scale) {
        this.yieldOverride = scale;
        setChanged();
    }

    public float getEffectiveYield() {
        if (!Float.isNaN(yieldOverride)) return Mth.clamp(yieldOverride, 0f, 1f);
        return loadedTier != null ? loadedTier.defaultYield : 0.5f;
    }

    // ── Launch ────────────────────────────────────────────────────────────

    /**
     * Spawns a missile entity and marks the mount as empty.
     *
     * @param launchId Telemetry UUID from the Python API; null for redstone-fired launches.
     * @return The spawned entity, or null if the launcher is empty / wrong side.
     */
    @Nullable
    public MissileEntity triggerLaunch(@Nullable UUID launchId) {
        if (loadedTier == null || level == null || level.isClientSide()) return null;
        if (!(level instanceof ServerLevel sLevel)) return null;

        double cx = worldPosition.getX() + 0.5;
        double cy = worldPosition.getY() + 1.0; // launch from top face
        double cz = worldPosition.getZ() + 0.5;

        MissileEntity missile = MissileEntity.create(
                sLevel,
                loadedTier,
                WarheadType.EXPLOSIVE,
                getEffectiveYield(),
                bearing,
                elevation,
                cx, cy, cz,
                targetPos,
                launchId
        );

        sLevel.addFreshEntity(missile);
        onLaunch();
        return missile;
    }

    // ── Redstone trigger ──────────────────────────────────────────────────

    public void onRedstoneChanged(boolean powered) {
        if (powered && !prevRedstonePowered) {
            triggerLaunch(null);
        }
        prevRedstonePowered = powered;
    }

    // ── Ticker ────────────────────────────────────────────────────────────

    public static <T extends BlockEntity> BlockEntityTicker<T> createTicker(Level level) {
        return null; // no continuous tick needed; redstone handled via neighborChanged
    }

    // ── Network sync ─────────────────────────────────────────────────────

    private void syncToClients() {
        if (!(level instanceof ServerLevel sLevel)) return;
        //? if >=26.1 {
        ChunkPos chunkPos = ChunkPos.containing(worldPosition);
        //?} else
        /*ChunkPos chunkPos = new ChunkPos(worldPosition);*/
        PacketDistributor.sendToPlayersTrackingChunk(sLevel, chunkPos,
                new LauncherStatePacket(worldPosition, bearing, elevation));
    }

    @Override
    public Packet<ClientGamePacketListener> getUpdatePacket() {
        return ClientboundBlockEntityDataPacket.create(this);
    }

    @Override
    public CompoundTag getUpdateTag(HolderLookup.Provider registries) {
        CompoundTag tag = new CompoundTag();
        tag.putFloat("bearing",   bearing);
        tag.putFloat("elevation", elevation);
        return tag;
    }

    // ── Save / Load ───────────────────────────────────────────────────────

    //? if >=26.1 {
    @Override
    protected void saveAdditional(ValueOutput output) {
        super.saveAdditional(output);
        output.putFloat("bearing",        bearing);
        output.putFloat("elevation",      elevation);
        output.putFloat("yieldOverride",  Float.isNaN(yieldOverride) ? -1f : yieldOverride);
        output.putBoolean("prevPowered",  prevRedstonePowered);
        if (loadedTier != null) output.putString("loadedTier", loadedTier.id);
        if (targetPos  != null) {
            output.putInt("targetX", targetPos.getX());
            output.putInt("targetY", targetPos.getY());
            output.putInt("targetZ", targetPos.getZ());
        }
    }

    @Override
    protected void loadAdditional(ValueInput input) {
        super.loadAdditional(input);
        bearing         = input.getFloatOr("bearing",       0f);
        elevation       = input.getFloatOr("elevation",     45f);
        float rawYield  = input.getFloatOr("yieldOverride", -1f);
        yieldOverride   = rawYield < 0 ? Float.NaN : rawYield;
        prevRedstonePowered = input.getBooleanOr("prevPowered", false);
        input.getString("loadedTier").ifPresent(id -> loadedTier = MissileTier.fromId(id));
        if (input.getInt("targetX").isPresent()) {
            targetPos = new BlockPos(
                    input.getIntOr("targetX", 0),
                    input.getIntOr("targetY", 0),
                    input.getIntOr("targetZ", 0));
        }
    }
    //?} else {
    /*@Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        tag.putFloat("bearing",       bearing);
        tag.putFloat("elevation",     elevation);
        tag.putFloat("yieldOverride", Float.isNaN(yieldOverride) ? -1f : yieldOverride);
        tag.putBoolean("prevPowered", prevRedstonePowered);
        if (loadedTier != null) tag.putString("loadedTier", loadedTier.id);
        if (targetPos != null) {
            tag.putInt("targetX", targetPos.getX());
            tag.putInt("targetY", targetPos.getY());
            tag.putInt("targetZ", targetPos.getZ());
        }
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        bearing   = tag.getFloat("bearing");
        elevation = tag.contains("elevation") ? tag.getFloat("elevation") : 45f;
        float rawYield = tag.contains("yieldOverride") ? tag.getFloat("yieldOverride") : -1f;
        yieldOverride  = rawYield < 0 ? Float.NaN : rawYield;
        prevRedstonePowered = tag.getBoolean("prevPowered");
        if (tag.contains("loadedTier")) loadedTier = MissileTier.fromId(tag.getString("loadedTier"));
        if (tag.contains("targetX")) {
            targetPos = new BlockPos(
                    tag.getInt("targetX"),
                    tag.getInt("targetY"),
                    tag.getInt("targetZ"));
        }
    }*/
    //?}
}
