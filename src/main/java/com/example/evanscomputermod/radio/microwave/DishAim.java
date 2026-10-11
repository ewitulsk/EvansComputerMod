package com.example.evanscomputermod.radio.microwave;

import java.util.function.UnaryOperator;

/**
 * Dish aiming maths, free of Minecraft so it is unit-testable.
 *
 * <p>Aim is (yaw, elevation) in degrees in the <i>block's</i> frame: yaw uses
 * Minecraft's convention (0 = south/+Z, 90 = west/-X, 180 = north, -90 = east),
 * elevation is positive upwards. On a Sable sub-level the block frame rotates
 * with the ship: {@code normal} maps a block-frame direction to world space
 * (identity on the ground), so the world boresight combines the aim with the
 * ship's orientation and a turning ship drags the beam off its target.
 */
public final class DishAim {
    private DishAim() {}

    /** Unit boresight in the block frame. */
    public static double[] direction(double yawDeg, double elevDeg) {
        double y = Math.toRadians(yawDeg), e = Math.toRadians(elevDeg);
        return new double[] {-Math.sin(y) * Math.cos(e), Math.sin(e), Math.cos(y) * Math.cos(e)};
    }

    /** The dish's "up" (local +Y) in the block frame: perpendicular to the boresight, towards the sky. */
    public static double[] up(double yawDeg, double elevDeg) {
        double y = Math.toRadians(yawDeg), e = Math.toRadians(elevDeg);
        return new double[] {Math.sin(y) * Math.sin(e), Math.cos(e), -Math.cos(y) * Math.sin(e)};
    }

    /** {yaw, elevation} (degrees) of a block-frame direction. */
    public static double[] aimOf(double[] dir) {
        double len = Math.sqrt(dot(dir, dir));
        if (len == 0) return new double[] {0, 0};
        double x = dir[0] / len, y = dir[1] / len, z = dir[2] / len;
        double yaw = Math.toDegrees(Math.atan2(-x, z));
        double elev = Math.toDegrees(Math.asin(Math.max(-1, Math.min(1, y))));
        return new double[] {normalizeYaw(yaw), elev};
    }

    public static double normalizeYaw(double yaw) {
        double y = yaw % 360;
        if (y > 180) y -= 360;
        if (y <= -180) y += 360;
        return y;
    }

    /**
     * World orientation quaternion {qx, qy, qz, qw} of a dish aimed at (yaw, elev)
     * in a block frame mapped to world by {@code normal}: local +Z is the boresight, +Y the dish's up.
     */
    public static float[] worldQuaternion(double yawDeg, double elevDeg, UnaryOperator<double[]> normal) {
        double[] f = unit(normal.apply(direction(yawDeg, elevDeg)));
        double[] u = normal.apply(up(yawDeg, elevDeg));
        double d = dot(u, f);
        u = unit(new double[] {u[0] - d * f[0], u[1] - d * f[1], u[2] - d * f[2]});
        double[] r = cross(u, f);   // local +X = Y x Z
        return fromBasis(r, u, f);
    }

    /** {yaw, elev} that points the dish along world direction {@code world}, through the inverse of {@code normal}. */
    public static double[] aimForWorld(double[] world, UnaryOperator<double[]> normal) {
        double[] bx = normal.apply(new double[] {1, 0, 0});
        double[] by = normal.apply(new double[] {0, 1, 0});
        double[] bz = normal.apply(new double[] {0, 0, 1});
        // normal is a rotation: its inverse is the transpose.
        return aimOf(new double[] {dot(bx, world), dot(by, world), dot(bz, world)});
    }

    /** Angle between two directions, degrees. */
    public static double angleDeg(double[] a, double[] b) {
        double c = dot(unit(a), unit(b));
        return Math.toDegrees(Math.acos(Math.max(-1, Math.min(1, c))));
    }

    static float[] fromBasis(double[] x, double[] y, double[] z) {
        double m00 = x[0], m01 = y[0], m02 = z[0];
        double m10 = x[1], m11 = y[1], m12 = z[1];
        double m20 = x[2], m21 = y[2], m22 = z[2];
        double tr = m00 + m11 + m22, qw, qx, qy, qz;
        if (tr > 0) {
            double s = Math.sqrt(tr + 1.0) * 2;
            qw = 0.25 * s; qx = (m21 - m12) / s; qy = (m02 - m20) / s; qz = (m10 - m01) / s;
        } else if (m00 > m11 && m00 > m22) {
            double s = Math.sqrt(1.0 + m00 - m11 - m22) * 2;
            qw = (m21 - m12) / s; qx = 0.25 * s; qy = (m01 + m10) / s; qz = (m02 + m20) / s;
        } else if (m11 > m22) {
            double s = Math.sqrt(1.0 + m11 - m00 - m22) * 2;
            qw = (m02 - m20) / s; qx = (m01 + m10) / s; qy = 0.25 * s; qz = (m12 + m21) / s;
        } else {
            double s = Math.sqrt(1.0 + m22 - m00 - m11) * 2;
            qw = (m10 - m01) / s; qx = (m02 + m20) / s; qy = (m12 + m21) / s; qz = 0.25 * s;
        }
        double n = Math.sqrt(qx * qx + qy * qy + qz * qz + qw * qw);
        return new float[] {(float) (qx / n), (float) (qy / n), (float) (qz / n), (float) (qw / n)};
    }

    static double dot(double[] a, double[] b) {
        return a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    }

    static double[] cross(double[] a, double[] b) {
        return new double[] {a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]};
    }

    static double[] unit(double[] a) {
        double n = Math.sqrt(dot(a, a));
        return n == 0 ? new double[] {0, 0, 1} : new double[] {a[0] / n, a[1] / n, a[2] / n};
    }
}
