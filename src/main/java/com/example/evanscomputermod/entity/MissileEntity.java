package com.example.evanscomputermod.entity;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.missile.MissileTelemetryStore;
import com.example.evanscomputermod.missile.MissileTier;
import com.example.evanscomputermod.missile.TelemetrySnapshot;
import com.example.evanscomputermod.missile.WarheadType;
import net.minecraft.core.BlockPos;
import net.minecraft.core.particles.ParticleTypes;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.world.damagesource.DamageSource;
import net.minecraft.network.syncher.EntityDataAccessor;
//? if >=26.1 {
import net.minecraft.world.level.storage.ValueInput;
import net.minecraft.world.level.storage.ValueOutput;
//?}
import net.minecraft.network.syncher.EntityDataSerializers;
import net.minecraft.network.syncher.SynchedEntityData;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.util.Mth;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.level.ClipContext;
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.HitResult;
import net.minecraft.world.phys.Vec3;

import org.jetbrains.annotations.Nullable;
import java.util.UUID;

/**
 * In-flight missile entity with a three-phase flight model:
 *
 * <ol>
 *   <li><b>Thrust</b> — Motor burn. Constant acceleration along the launch
 *       vector until {@code thrustTicks} is exhausted.</li>
 *   <li><b>Coast / Guiding</b> — Motor off. Gravity and drag act on the
 *       missile. GPS-guided missiles additionally apply a per-tick steering
 *       correction capped at {@code tier.turnRate} degrees/tick.</li>
 *   <li><b>Impact / Lost</b> — Terminal states. Triggers warhead detonation
 *       or silent discard; updates telemetry store with final status.</li>
 * </ol>
 *
 * Telemetry is written to {@link MissileTelemetryStore} every tick so the
 * Python API can poll it without any extra network packets.
 */
public class MissileEntity extends Entity {

    // ── Phase constants (byte, synced to clients for particle rendering) ──
    public static final byte PHASE_THRUST   = 0;
    public static final byte PHASE_COAST    = 1;
    public static final byte PHASE_GUIDING  = 2;
    public static final byte PHASE_IMPACT   = 3;
    public static final byte PHASE_LOST     = 4;

    private static final EntityDataAccessor<Byte> PHASE_DATA =
            SynchedEntityData.defineId(MissileEntity.class, EntityDataSerializers.BYTE);

    // ── Physics constants ──
    private static final double GRAVITY       = 0.05;  // blocks/tick² downward
    private static final double AIR_DRAG      = 0.99;  // velocity multiplier per tick
    private static final double MIN_SPEED     = 0.01;  // below this, treat as stationary

    // ── Server-side state (not synced; not needed on client) ──
    private MissileTier tier = MissileTier.MRBM;
    private WarheadType warheadType = WarheadType.EXPLOSIVE;
    private float yieldScale = MissileTier.MRBM.defaultYield;

    /** Pre-computed acceleration magnitude per thrust tick. */
    private double thrustAccel = 0.0;
    /** Remaining thrust ticks. */
    private int remainingThrust = 0;
    /** Remaining guidance-fuel ticks (0 = coast only). */
    private int guidanceFuel = 0;
    /** GPS target world position, or null for unguided / fuel-exhausted. */
    @Nullable private BlockPos targetPos = null;
    /** Telemetry tracking UUID; null if not launched via Python API. */
    @Nullable private UUID launchId = null;
    /** Total guidance fuel at launch (for fuel-pct calculation). */
    private int totalGuidanceFuel = 0;

    // ── Constructor required by EntityType.Builder ──
    public MissileEntity(EntityType<?> type, Level level) {
        super(type, level);
        //? if <=1.21.1 {
        /*this.noCulling = true;*/
        //?}
    }

    // ──────────────────────────────────────────────────────────────────────
    // Factory
    // ──────────────────────────────────────────────────────────────────────

    /**
     * Creates and fully configures a missile entity ready to be added to the world.
     *
     * @param level       Server level.
     * @param tier        Missile tier (determines physics parameters).
     * @param warhead     Warhead type.
     * @param yieldScale  Warhead yield 0.0–1.0.
     * @param bearingDeg  Launch bearing in degrees (0 = North, 90 = East).
     * @param elevDeg     Launch elevation in degrees above horizontal (5–85).
     * @param launchX     World X of the launch point.
     * @param launchY     World Y of the launch point.
     * @param launchZ     World Z of the launch point.
     * @param targetPos   GPS target block position, or null for unguided.
     * @param launchId    UUID used as telemetry key; null if no telemetry needed.
     */
    public static MissileEntity create(
            ServerLevel level,
            MissileTier tier,
            WarheadType warhead,
            float yieldScale,
            float bearingDeg,
            float elevDeg,
            double launchX, double launchY, double launchZ,
            @Nullable BlockPos targetPos,
            @Nullable UUID launchId) {

        MissileEntity missile = new MissileEntity(ModEntities.MISSILE.get(), level);
        missile.tier        = tier;
        missile.warheadType = warhead;
        missile.yieldScale  = yieldScale;
        missile.targetPos   = targetPos;
        missile.launchId    = launchId;

        // Compute acceleration: ramp from 0 to topSpeed over thrustTicks
        missile.thrustAccel     = tier.topSpeed / tier.thrustTicks;
        missile.remainingThrust = tier.thrustTicks;

        // Guidance fuel = enough to cover max range at top speed, with 50% margin
        int fuelTicks = (int) (tier.maxRange / tier.topSpeed * 1.5);
        if (targetPos != null) {
            // Scale to actual target distance with same 50% margin
            double dist = Math.sqrt(
                    Math.pow(targetPos.getX() - launchX, 2) +
                    Math.pow(targetPos.getY() - launchY, 2) +
                    Math.pow(targetPos.getZ() - launchZ, 2));
            fuelTicks = (int) (dist / tier.topSpeed * 1.5);
        }
        missile.guidanceFuel      = fuelTicks;
        missile.totalGuidanceFuel = fuelTicks;

        // Set initial position slightly above the launch block center
        missile.setPos(launchX, launchY + 0.5, launchZ);

        // Pre-compute initial velocity direction (zero velocity at launch; thrust will build it)
        // Store launch direction as the starting delta movement so the entity faces correctly
        Vec3 dir = bearingElevToDir(bearingDeg, elevDeg);
        missile.setDeltaMovement(dir.scale(missile.thrustAccel));

        // Set visual rotation to match launch direction
        missile.setYRot(bearingDeg);
        missile.setXRot(-elevDeg);

        missile.setPhase(PHASE_THRUST);
        return missile;
    }

    // ──────────────────────────────────────────────────────────────────────
    // Tick — server-side flight loop
    // ──────────────────────────────────────────────────────────────────────

    @Override
    public void tick() {
        super.tick();

        if (level().isClientSide()) {
            tickClient();
            return;
        }

        byte phase = getPhase();

        // ── Phase: Thrust ────────────────────────────────────────────────
        if (phase == PHASE_THRUST) {
            if (remainingThrust > 0) {
                // Accelerate along current direction of travel
                Vec3 vel   = getDeltaMovement();
                double speed = vel.length();
                Vec3 dir   = speed > MIN_SPEED ? vel.normalize() : bearingElevToDir(getYRot(), -getXRot());
                Vec3 newVel = dir.scale(Math.min(speed + thrustAccel, tier.topSpeed));
                setDeltaMovement(newVel);
                remainingThrust--;

                if (remainingThrust == 0) {
                    setPhase(targetPos != null && guidanceFuel > 0 ? PHASE_GUIDING : PHASE_COAST);
                }
            }

            // Apply light drag but no gravity during thrust
            setDeltaMovement(getDeltaMovement().scale(AIR_DRAG));
            spawnTrailParticles(true);
        }

        // ── Phase: Coast ─────────────────────────────────────────────────
        else if (phase == PHASE_COAST) {
            applyGravityAndDrag();
            spawnTrailParticles(false);
        }

        // ── Phase: Guiding ───────────────────────────────────────────────
        else if (phase == PHASE_GUIDING) {
            if (targetPos != null && guidanceFuel > 0) {
                steerTowardTarget();
                guidanceFuel--;
                if (guidanceFuel == 0) {
                    setPhase(PHASE_COAST);
                }
            } else {
                setPhase(PHASE_COAST);
            }
            applyGravityAndDrag();
            spawnTrailParticles(false);
        }

        // Dead phases — should not tick further, but guard just in case
        else {
            return;
        }

        // ── Collision detection via ray cast ──────────────────────────────
        Vec3 from = position();
        Vec3 vel  = getDeltaMovement();
        Vec3 to   = from.add(vel);

        BlockHitResult hit = level().clip(new ClipContext(
                from, to,
                ClipContext.Block.COLLIDER,
                ClipContext.Fluid.NONE,
                this));

        if (hit.getType() != HitResult.Type.MISS) {
            onImpact(hit.getBlockPos(), hit.getLocation());
            return;
        }

        // ── Move and update rotation ──────────────────────────────────────
        Vec3 newPos = from.add(vel);

        // Lost if fallen below world floor
        //? if >=26.1 {
        if (newPos.y < level().getMinY() - 64) {
        //?} else {
        /*if (newPos.y < level().getMinBuildHeight() - 64) {*/
        //?}
            setPhase(PHASE_LOST);
            publishFinalTelemetry("lost");
            discard();
            return;
        }

        setPos(newPos.x, newPos.y, newPos.z);
        updateRotationFromVelocity();

        // ── Telemetry ─────────────────────────────────────────────────────
        if (launchId != null) {
            publishTelemetry();
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // Impact
    // ──────────────────────────────────────────────────────────────────────

    private void onImpact(BlockPos hitBlock, Vec3 hitLocation) {
        setPhase(PHASE_IMPACT);
        publishFinalTelemetry("impact");

        if (!level().isClientSide()) {
            warheadType.onImpact(level(), hitLocation.x, hitLocation.y, hitLocation.z, yieldScale);
        }
        discard();
    }

    // ──────────────────────────────────────────────────────────────────────
    // Physics helpers
    // ──────────────────────────────────────────────────────────────────────

    private void applyGravityAndDrag() {
        setDeltaMovement(getDeltaMovement().add(0, -GRAVITY, 0).scale(AIR_DRAG));
    }

    /**
     * Rotates the current velocity vector toward the GPS target by at most
     * {@code tier.turnRate} degrees this tick.
     */
    private void steerTowardTarget() {
        if (targetPos == null) return;

        Vec3 pos    = position();
        Vec3 vel    = getDeltaMovement();
        double speed = vel.length();
        if (speed < MIN_SPEED) return;

        Vec3 currentDir = vel.normalize();
        Vec3 toTarget   = new Vec3(
                targetPos.getX() + 0.5 - pos.x,
                targetPos.getY() + 0.5 - pos.y,
                targetPos.getZ() + 0.5 - pos.z
        ).normalize();

        double dot = Mth.clamp(currentDir.dot(toTarget), -1.0, 1.0);
        double angleBetween = Math.acos(dot); // radians

        if (angleBetween < 0.001) {
            // Already aligned — maintain speed
            setDeltaMovement(toTarget.scale(speed));
            return;
        }

        double maxAngleRad = Math.toRadians(tier.turnRate);
        double t = Math.min(1.0, maxAngleRad / angleBetween);

        // Linear interpolation of directions (normalised) — good approximation for small angles
        Vec3 newDir = new Vec3(
                currentDir.x + (toTarget.x - currentDir.x) * t,
                currentDir.y + (toTarget.y - currentDir.y) * t,
                currentDir.z + (toTarget.z - currentDir.z) * t
        ).normalize();

        setDeltaMovement(newDir.scale(speed));
    }

    /** Converts compass bearing + elevation to a unit direction vector. */
    static Vec3 bearingElevToDir(float bearingDeg, float elevDeg) {
        double bRad = Math.toRadians(bearingDeg);
        double eRad = Math.toRadians(elevDeg);
        double cosE = Math.cos(eRad);
        return new Vec3(
                Math.sin(bRad) * cosE,   // X: East positive
                Math.sin(eRad),           // Y: Up positive
                -Math.cos(bRad) * cosE   // Z: North negative
        );
    }

    private void updateRotationFromVelocity() {
        Vec3 vel = getDeltaMovement();
        if (vel.horizontalDistance() > MIN_SPEED || Math.abs(vel.y) > MIN_SPEED) {
            setYRot((float) Math.toDegrees(Math.atan2(vel.x, -vel.z)));
            setXRot((float) -Math.toDegrees(Math.atan2(vel.y, vel.horizontalDistance())));
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // Trail particles (server-side broadcast)
    // ──────────────────────────────────────────────────────────────────────

    private void spawnTrailParticles(boolean thrust) {
        if (!(level() instanceof ServerLevel sLevel)) return;
        Vec3 pos = position();
        Vec3 vel = getDeltaMovement().scale(-0.5); // emit behind the missile

        if (thrust) {
            sLevel.sendParticles(ParticleTypes.FLAME,
                    pos.x, pos.y, pos.z, 3,
                    vel.x * 0.5, vel.y * 0.5, vel.z * 0.5, 0.05);
        }
        sLevel.sendParticles(ParticleTypes.SMOKE,
                pos.x, pos.y, pos.z, 2,
                vel.x * 0.3, vel.y * 0.3, vel.z * 0.3, 0.02);
    }

    // ──────────────────────────────────────────────────────────────────────
    // Client tick (particle effects driven by synced phase)
    // ──────────────────────────────────────────────────────────────────────

    private void tickClient() {
        // Client-side particles are handled by the renderer / shader layer.
        // The server already broadcasts particles; this is a no-op for now.
    }

    // ──────────────────────────────────────────────────────────────────────
    // Telemetry
    // ──────────────────────────────────────────────────────────────────────

    private void publishTelemetry() {
        if (launchId == null) return;
        Vec3 vel = getDeltaMovement();
        Vec3 pos = position();

        float fuelPct = totalGuidanceFuel > 0
                ? (float) guidanceFuel / totalGuidanceFuel
                : 1.0f;

        float tta = -1f;
        if (targetPos != null) {
            double dist = pos.distanceTo(new Vec3(
                    targetPos.getX() + 0.5,
                    targetPos.getY() + 0.5,
                    targetPos.getZ() + 0.5));
            double speed = vel.length();
            if (speed > MIN_SPEED) {
                tta = (float) (dist / speed / 20.0); // ticks → seconds
            }
        }

        String statusName = switch (getPhase()) {
            case PHASE_THRUST  -> "thrust";
            case PHASE_COAST   -> "coast";
            case PHASE_GUIDING -> "guiding";
            case PHASE_IMPACT  -> "impact";
            default            -> "lost";
        };

        MissileTelemetryStore.put(launchId, new TelemetrySnapshot(
                pos.x, pos.y, pos.z,
                vel.x, vel.y, vel.z,
                fuelPct, statusName, tta));
    }

    private void publishFinalTelemetry(String status) {
        if (launchId == null) return;
        Vec3 pos = position();
        Vec3 vel = getDeltaMovement();
        MissileTelemetryStore.put(launchId, new TelemetrySnapshot(
                pos.x, pos.y, pos.z,
                vel.x, vel.y, vel.z,
                0f, status, 0f));
    }

    // ──────────────────────────────────────────────────────────────────────
    // Synced data
    // ──────────────────────────────────────────────────────────────────────

    @Override
    protected void defineSynchedData(SynchedEntityData.Builder builder) {
        builder.define(PHASE_DATA, PHASE_THRUST);
    }

    public byte getPhase() {
        return entityData.get(PHASE_DATA);
    }

    public void setPhase(byte phase) {
        entityData.set(PHASE_DATA, phase);
    }

    // ──────────────────────────────────────────────────────────────────────
    // NBT persistence
    // ──────────────────────────────────────────────────────────────────────

    //? if >=26.1 {
    @Override
    protected void addAdditionalSaveData(ValueOutput output) {
        output.putString("tier",             tier.id);
        output.putString("warhead",          warheadType.name());
        output.putFloat("yieldScale",        yieldScale);
        output.putInt("remainingThrust",     remainingThrust);
        output.putInt("guidanceFuel",        guidanceFuel);
        output.putInt("totalGuidanceFuel",   totalGuidanceFuel);
        output.putDouble("thrustAccel",      thrustAccel);
        output.putByte("phase",              getPhase());
        if (targetPos != null) {
            output.putInt("targetX", targetPos.getX());
            output.putInt("targetY", targetPos.getY());
            output.putInt("targetZ", targetPos.getZ());
        }
        if (launchId != null) {
            output.putLong("launchIdMost",   launchId.getMostSignificantBits());
            output.putLong("launchIdLeast",  launchId.getLeastSignificantBits());
        }
    }

    @Override
    protected void readAdditionalSaveData(ValueInput input) {
        tier              = MissileTier.fromId(input.getStringOr("tier", "mrbm"));
        warheadType       = WarheadType.fromId(input.getStringOr("warhead", "EXPLOSIVE"));
        yieldScale        = input.getFloatOr("yieldScale",        (float) MissileTier.MRBM.defaultYield);
        remainingThrust   = input.getIntOr("remainingThrust",     0);
        guidanceFuel      = input.getIntOr("guidanceFuel",        0);
        totalGuidanceFuel = input.getIntOr("totalGuidanceFuel",   guidanceFuel);
        thrustAccel       = input.getDoubleOr("thrustAccel",      0.0);
        setPhase(input.getByteOr("phase",                         PHASE_THRUST));
        if (input.getInt("targetX").isPresent()) {
            targetPos = new BlockPos(
                    input.getIntOr("targetX", 0),
                    input.getIntOr("targetY", 0),
                    input.getIntOr("targetZ", 0));
        }
        if (input.getLong("launchIdMost").isPresent()) {
            launchId = new UUID(
                    input.getLongOr("launchIdMost",  0L),
                    input.getLongOr("launchIdLeast", 0L));
        }
    }
    //?} else {
    /*@Override
    protected void addAdditionalSaveData(CompoundTag tag) {
        tag.putString("tier",        tier.id);
        tag.putString("warhead",     warheadType.name());
        tag.putFloat("yieldScale",   yieldScale);
        tag.putInt("remainingThrust",remainingThrust);
        tag.putInt("guidanceFuel",   guidanceFuel);
        tag.putInt("totalGuidanceFuel", totalGuidanceFuel);
        tag.putDouble("thrustAccel", thrustAccel);
        tag.putByte("phase",         getPhase());
        if (targetPos != null) {
            tag.putInt("targetX", targetPos.getX());
            tag.putInt("targetY", targetPos.getY());
            tag.putInt("targetZ", targetPos.getZ());
        }
        if (launchId != null) {
            tag.putLong("launchIdMost",  launchId.getMostSignificantBits());
            tag.putLong("launchIdLeast", launchId.getLeastSignificantBits());
        }
    }

    @Override
    protected void readAdditionalSaveData(CompoundTag tag) {
        tier         = MissileTier.fromId(tag.getString("tier"));
        warheadType  = WarheadType.fromId(tag.getString("warhead"));
        yieldScale   = tag.getFloat("yieldScale");
        remainingThrust = tag.getInt("remainingThrust");
        guidanceFuel    = tag.getInt("guidanceFuel");
        totalGuidanceFuel = tag.contains("totalGuidanceFuel") ? tag.getInt("totalGuidanceFuel") : guidanceFuel;
        thrustAccel  = tag.getDouble("thrustAccel");
        setPhase(tag.getByte("phase"));
        if (tag.contains("targetX")) {
            targetPos = new BlockPos(
                    tag.getInt("targetX"),
                    tag.getInt("targetY"),
                    tag.getInt("targetZ"));
        }
        if (tag.contains("launchIdMost")) {
            launchId = new UUID(
                    tag.getLong("launchIdMost"),
                    tag.getLong("launchIdLeast"));
        }
    }*/
    //?}

    // ──────────────────────────────────────────────────────────────────────
    // Misc Entity overrides
    // ──────────────────────────────────────────────────────────────────────

    @Override
    public boolean isPickable() { return false; }

    @Override
    public boolean isPushable() { return false; }

    //? if >=26.1 {
    @Override
    public boolean hurtServer(ServerLevel level, DamageSource source, float amount) { return false; }
    //?}

    @Override
    public boolean isPushedByFluid() { return false; }

    /** Public getter so the launcher / Python module can access tier for display. */
    public MissileTier getTier() { return tier; }

    /** Exposes launchId so interceptors (future CIWS block) can cancel tracking. */
    @Nullable public UUID getLaunchId() { return launchId; }
}
