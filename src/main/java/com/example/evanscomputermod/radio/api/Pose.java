package com.example.evanscomputermod.radio.api;

/**
 * Where an antenna is and which way it points, in world space: dimension id,
 * position in metres (blocks) and orientation as a unit quaternion. Endpoints
 * on Sable sub-levels report their projected world pose here, so the medium
 * never needs to know about ships.
 */
public record Pose(String dimension, double x, double y, double z, float qx, float qy, float qz, float qw) {

    public static Pose at(String dimension, double x, double y, double z) {
        return new Pose(dimension, x, y, z, 0, 0, 0, 1);
    }

    public double distanceTo(Pose o) {
        double dx = x - o.x, dy = y - o.y, dz = z - o.z;
        return Math.sqrt(dx * dx + dy * dy + dz * dz);
    }

    public boolean sameDimension(Pose o) {
        return dimension.equals(o.dimension);
    }

    /** Rotate a local direction (antenna frame) into world space. */
    public double[] toWorld(double lx, double ly, double lz) {
        // v' = q v q*, expanded.
        double tx = 2 * (qy * lz - qz * ly), ty = 2 * (qz * lx - qx * lz), tz = 2 * (qx * ly - qy * lx);
        return new double[] {
            lx + qw * tx + (qy * tz - qz * ty),
            ly + qw * ty + (qz * tx - qx * tz),
            lz + qw * tz + (qx * ty - qy * tx)
        };
    }

    /** Rotate a world direction into the antenna's local frame. */
    public double[] toLocal(double wx, double wy, double wz) {
        return new Pose(dimension, x, y, z, -qx, -qy, -qz, qw).toWorld(wx, wy, wz);
    }

    /** True if moving from this pose to {@code o} is past the given thresholds (metres, radians). */
    public boolean movedBeyond(Pose o, double metres, double radians) {
        if (!sameDimension(o) || distanceTo(o) > metres) return true;
        double dot = Math.abs(qx * o.qx + qy * o.qy + qz * o.qz + qw * o.qw);
        return 2 * Math.acos(Math.min(1, dot)) > radians;
    }
}
