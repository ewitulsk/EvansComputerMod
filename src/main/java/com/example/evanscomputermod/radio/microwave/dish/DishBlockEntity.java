package com.example.evanscomputermod.radio.microwave.dish;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import com.example.evanscomputermod.radio.microwave.DishAim;
import com.example.evanscomputermod.radio.microwave.DishPattern;
import com.example.evanscomputermod.radio.microwave.MicrowaveContent;
import com.example.evanscomputermod.radio.microwave.MicrowaveLink;
import com.example.evanscomputermod.sensor.SensorSable;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.Vec3;
import org.jetbrains.annotations.Nullable;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.function.UnaryOperator;

/**
 * The dish's controller: its aim (yaw/elevation in the block's frame, see
 * {@link DishAim}) and its world pose. On a Sable sub-level the pose goes
 * through the sub-level's transform, so the beam turns with the ship; a
 * computer holds a link by re-aiming with {@code aim_at} or {@code align}.
 */
public class DishBlockEntity extends BlockEntity {
    private double yaw, elevation;
    private volatile @Nullable MicrowaveLink radio;
    private long radioSeen = Long.MIN_VALUE;
    private final DishPeripheral peripheral = new DishPeripheral(this);

    public DishBlockEntity(BlockPos pos, BlockState state) {
        super(MicrowaveContent.DISH_BE.get(), pos, state);
        resetAim(state.hasProperty(DishBlock.FACING) ? state.getValue(DishBlock.FACING) : Direction.NORTH);
    }

    public DishPeripheral getPeripheral() {
        return peripheral;
    }

    public DishSize size() {
        return getBlockState().getBlock() instanceof DishBlock d ? d.size() : DishSize.SMALL;
    }

    public Direction facing() {
        return getBlockState().getValue(DishBlock.FACING);
    }

    public double yaw() { return yaw; }
    public double elevation() { return elevation; }

    /** Point straight out of the mount. */
    public void resetAim(Direction facing) {
        yaw = DishAim.normalizeYaw(facing.toYRot());
        elevation = 0;
        setChanged();
    }

    public void setAim(double yawDeg, double elevDeg) {
        if (!Double.isFinite(yawDeg) || !Double.isFinite(elevDeg)) throw new IllegalArgumentException("aim must be finite");
        if (elevDeg < -90 || elevDeg > 90) throw new IllegalArgumentException("pitch must be -90 to 90 degrees");
        yaw = DishAim.normalizeYaw(yawDeg);
        elevation = elevDeg;
        setChanged();
    }

    public void nudge(double dYaw, double dElev) {
        setAim(yaw + dYaw, Math.max(-90, Math.min(90, elevation + dElev)));
    }

    /** Half a beamwidth at the connected radio's frequency, else one degree. */
    public double nudgeStepDeg() {
        MicrowaveLink r = radio();
        DishPattern p = r == null ? null : r.dish();
        return p == null ? 1.0 : Math.max(0.01, Math.min(2.0, p.beamwidthDeg() / 2));
    }

    // ------------------------------------------------------------ geometry

    /** Centre of the dish face, in the block's own (plot) coordinates. */
    public Vec3 centreInBlockSpace() {
        DishSize s = size();
        Direction right = facing().getClockWise();
        return Vec3.atCenterOf(worldPosition)
                .add(right.getStepX() * s.centreRight(), s.centreUp(), right.getStepZ() * s.centreRight());
    }

    /** Block-frame to world rotation (the Sable sub-level's, identity on the ground). */
    UnaryOperator<double[]> frameNormal() {
        if (level == null) return v -> v;
        SensorSable.Frame f = SensorSable.frame(level, centreInBlockSpace());
        return v -> {
            Vec3 r = f.normal(new Vec3(v[0], v[1], v[2]));
            return new double[] {r.x, r.y, r.z};
        };
    }

    /** The dish's world pose if it were aimed at (yaw, elevation). */
    public @Nullable Pose poseFor(double yawDeg, double elevDeg) {
        return poseFor(yawDeg, elevDeg, frameNormal());
    }

    private @Nullable Pose poseFor(double yawDeg, double elevDeg, UnaryOperator<double[]> normal) {
        if (level == null) return null;
        Vec3 p = SensorSable.toWorld(level, centreInBlockSpace());
        float[] q = DishAim.worldQuaternion(yawDeg, elevDeg, normal);
        return new Pose(level.dimension().location().toString(), p.x, p.y, p.z, q[0], q[1], q[2], q[3]);
    }

    public @Nullable Pose worldPose() {
        return poseFor(yaw, elevation);
    }

    /** World boresight direction. */
    public double[] worldBoresight() {
        return frameNormal().apply(DishAim.direction(yaw, elevation));
    }

    /** Aim at a world point (accounts for the sub-level's rotation). */
    public void aimAt(double x, double y, double z) {
        if (level == null) return;
        Vec3 c = SensorSable.toWorld(level, centreInBlockSpace());
        double[] dir = {x - c.x, y - c.y, z - c.z};
        if (dir[0] == 0 && dir[1] == 0 && dir[2] == 0) throw new IllegalArgumentException("that is the dish itself");
        double[] aim = DishAim.aimForWorld(dir, frameNormal());
        setAim(aim[0], aim[1]);
    }

    // ------------------------------------------------------------ radio

    /** Called every tick by the microwave radio feeding this dish. */
    public void linkRadio(MicrowaveLink link) {
        radio = link;
        radioSeen = level == null ? 0 : level.getGameTime();
    }

    /** The radio feeding this dish, if one ticked in the last few ticks. */
    public @Nullable MicrowaveLink radio() {
        if (level == null || level.getGameTime() - radioSeen > 5) return null;
        return radio;
    }

    public String radioSummary() {
        MicrowaveLink r = radio();
        if (r == null) return "no radio";
        Map<String, Object> s = r.status();
        return String.format("%d GHz ch %d, %s", r.band().ghz, r.channelNumber(),
                Boolean.TRUE.equals(s.get("linked")) ? String.format("RSSI %.1f dBm", (Double) s.get("rssi_dbm")) : "no link");
    }

    /**
     * Scan around the current aim for the strongest signal from any radio on
     * this channel (a coarse-to-fine grid, 21×21 points per pass, each pass a
     * fifth the size of the last, down to 1/20 of a beamwidth). Takes the best
     * aim only if it is above the receiver's sensitivity.
     */
    public Map<String, Object> align(double spanDeg) {
        MicrowaveLink link = radio();
        if (link == null || link.dish() == null) throw new IllegalStateException("no microwave radio connected to this dish");
        if (!(spanDeg > 0 && spanDeg <= 90)) throw new IllegalArgumentException("span must be 0-90 degrees");
        RadioMedium m = RadioMediumHooks.medium();
        List<MicrowaveLink> others = link.sameChannelRadios();
        UnaryOperator<double[]> normal = frameNormal();
        double stop = link.dish().beamwidthDeg() / 20;
        double cy = yaw, ce = elevation, best = Double.NEGATIVE_INFINITY, step = spanDeg / 10;
        int evaluated = 0;
        while (!others.isEmpty()) {
            double by = cy, be = ce;
            for (int i = -10; i <= 10; i++) {
                for (int j = -10; j <= 10; j++) {
                    double y = cy + i * step, e = Math.max(-90, Math.min(90, ce + j * step));
                    Pose p = poseFor(y, e, normal);
                    for (MicrowaveLink o : others) {
                        double v = link.predictedRxDbm(o, p, m);
                        evaluated++;
                        if (v > best) {
                            best = v;
                            by = y;
                            be = e;
                        }
                    }
                }
            }
            cy = by;
            ce = be;
            if (step <= stop) break;
            step = Math.max(stop, step / 5);
        }
        Map<String, Object> r = new LinkedHashMap<>();
        boolean found = best >= link.sensitivityDbm();
        if (found) setAim(cy, ce);
        r.put("found", found);
        r.put("rssi_dbm", best);
        r.put("yaw", yaw);
        r.put("pitch", elevation);
        r.put("radios", others.size());
        r.put("points", evaluated);
        return r;
    }

    // ------------------------------------------------------------ persistence

    @Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        tag.putDouble("Yaw", yaw);
        tag.putDouble("Elevation", elevation);
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        if (tag.contains("Yaw")) yaw = DishAim.normalizeYaw(tag.getDouble("Yaw"));
        if (tag.contains("Elevation")) elevation = Math.max(-90, Math.min(90, tag.getDouble("Elevation")));
    }
}
//?}
