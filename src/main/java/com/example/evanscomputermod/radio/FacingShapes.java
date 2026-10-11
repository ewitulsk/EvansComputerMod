package com.example.evanscomputermod.radio;

//? if <=1.21.1 {
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.phys.shapes.Shapes;
import net.minecraft.world.phys.shapes.VoxelShape;

import java.util.EnumMap;
import java.util.Map;

/**
 * Shapes for blocks whose model faces north and is turned by the blockstate's
 * {@code y} rotation: the same boxes turned to each horizontal facing, so the
 * outline and collision follow the model (antennas, jacks and ports included).
 */
public final class FacingShapes {
    private FacingShapes() {}

    /**
     * {@code boxes} are {x0, y0, z0, x1, y1, z1} in pixels for the north-facing
     * model; returns one shape per horizontal facing.
     */
    public static Map<Direction, VoxelShape> of(double[]... boxes) {
        Map<Direction, VoxelShape> out = new EnumMap<>(Direction.class);
        for (Direction facing : Direction.Plane.HORIZONTAL) {
            VoxelShape s = Shapes.empty();
            for (double[] b : boxes) s = Shapes.or(s, turned(b, facing));
            out.put(facing, s.optimize());
        }
        return out;
    }

    /** One north-model box turned to {@code facing} (clockwise seen from above, like the blockstate's y rotation). */
    public static VoxelShape turned(double[] b, Direction facing) {
        double x0 = b[0], y0 = b[1], z0 = b[2], x1 = b[3], y1 = b[4], z1 = b[5];
        return switch (facing) {
            case EAST -> Block.box(16 - z1, y0, x0, 16 - z0, y1, x1);
            case SOUTH -> Block.box(16 - x1, y0, 16 - z1, 16 - x0, y1, 16 - z0);
            case WEST -> Block.box(z0, y0, 16 - x1, z1, y1, 16 - x0);
            default -> Block.box(x0, y0, z0, x1, y1, z1);
        };
    }

    public static double[] box(double x0, double y0, double z0, double x1, double y1, double z1) {
        return new double[] {x0, y0, z0, x1, y1, z1};
    }
}
//?}
