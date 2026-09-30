package com.example.evanscomputermod.sensor;

//? if <=1.21.1 {
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.AttachFace;
import net.minecraft.world.phys.Vec3;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Where a sensor sits relative to the computer it is wired to. Both are on
 * the same structure (or both in the world), so this is exact and never
 * changes while the vehicle moves.
 *
 * <p>Computer frame: x = the computer's facing (the way its screen looks),
 * y = left of that, z = up; the origin is the computer block's centre. Units
 * are blocks.
 */
public record SensorMount(BlockPos sensorPos, Vec3 origin, Direction sensorForward, AttachFace face,
                          Vec3 computerCentre, Direction computerForward) {

    public static final Vec3 UP = new Vec3(0, 1, 0);

    public static SensorMount of(BlockPos computerPos, Direction computerFacing, BlockPos sensorPos, BlockState sensorState) {
        Vec3 origin = Vec3.atLowerCornerOf(sensorPos).add(LidarSensorBlock.headCentre(sensorState));
        return new SensorMount(sensorPos, origin, LidarSensorBlock.forward(sensorState),
                sensorState.getValue(LidarSensorBlock.FACE), Vec3.atCenterOf(computerPos), computerFacing);
    }

    static Vec3 vec(Direction d) {
        return new Vec3(d.getStepX(), d.getStepY(), d.getStepZ());
    }

    static Vec3 left(Direction forward) {
        return UP.cross(vec(forward));
    }

    /** Sensor-frame direction (forward, left, up) in block (plot) axes. */
    public Vec3 sensorToBlock(double f, double l, double u) {
        return vec(sensorForward).scale(f).add(left(sensorForward).scale(l)).add(UP.scale(u));
    }

    /** A block-axes vector in the computer frame (x forward, y left, z up). */
    public Vec3 blockToComputer(Vec3 v) {
        Vec3 f = vec(computerForward);
        return new Vec3(v.dot(f), v.dot(left(computerForward)), v.y);
    }

    /** Sensor origin in the computer frame. */
    public Vec3 offset() {
        return blockToComputer(origin.subtract(computerCentre));
    }

    /** Sensor forward relative to the computer's forward, degrees, counter-clockwise from above. */
    public double yaw() {
        Vec3 f = vec(sensorForward);
        return Math.toDegrees(Math.atan2(f.dot(left(computerForward)), f.dot(vec(computerForward))));
    }

    /** Integer block offset in the computer frame; stable across moves and rotations. */
    public String key() {
        Vec3 o = blockToComputer(Vec3.atLowerCornerOf(sensorPos).subtract(computerCentre.subtract(0.5, 0.5, 0.5)));
        return Math.round(o.x) + "," + Math.round(o.y) + "," + Math.round(o.z) + "," + face.getSerializedName()
                + "," + Math.round(yaw());
    }

    public Map<String, Object> toMap() {
        Map<String, Object> m = new LinkedHashMap<>();
        Vec3 o = offset();
        m.put("x", o.x);
        m.put("y", o.y);
        m.put("z", o.z);
        m.put("yaw", yaw());
        m.put("mount", face.getSerializedName());
        Vec3 rel = Vec3.atLowerCornerOf(sensorPos).subtract(computerCentre.subtract(0.5, 0.5, 0.5));
        m.put("block", List.of((int) Math.round(rel.x), (int) Math.round(rel.y), (int) Math.round(rel.z)));
        m.put("facing", sensorForward.getSerializedName());
        return m;
    }
}
//?}
