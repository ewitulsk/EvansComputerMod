package com.example.evanscomputermod.computer.overlay;

//? if <=1.21.1 {

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.phys.Vec3;
import org.jetbrains.annotations.Nullable;

/**
 * Where a point on a Screen cluster's face lands in its framebuffer. The
 * cluster's picture starts at the anchor block's top-left corner (as the
 * viewer sees it) and spans {@code cols x rows} blocks; this matches
 * {@code ScreenBlockEntityRenderer}.
 */
public final class ScreenGeometry {

    private ScreenGeometry() {
    }

    /** Top-left corner of the anchor's face, block-local. */
    public static Vec3 topLeft(Direction facing) {
        return switch (facing) {
            case SOUTH -> new Vec3(0, 1, 1);
            case WEST -> new Vec3(0, 1, 0);
            case EAST -> new Vec3(1, 1, 1);
            default -> new Vec3(1, 1, 0);
        };
    }

    /** Unit vector along the face to the viewer's right. */
    public static Vec3 right(Direction facing) {
        return switch (facing) {
            case SOUTH -> new Vec3(1, 0, 0);
            case WEST -> new Vec3(0, 0, 1);
            case EAST -> new Vec3(0, 0, -1);
            default -> new Vec3(-1, 0, 0);
        };
    }

    /** Framebuffer pixel {@code {x, y}} under world point {@code hit}, or null if it's off the picture. */
    @Nullable
    public static int[] pixelAt(Direction facing, BlockPos anchor, int cols, int rows, int gfxW, int gfxH, Vec3 hit) {
        if (cols <= 0 || rows <= 0 || gfxW <= 0 || gfxH <= 0) return null;
        Vec3 tl = Vec3.atLowerCornerOf(anchor).add(topLeft(facing));
        Vec3 d = hit.subtract(tl);
        double u = d.dot(right(facing));
        double v = -d.y;
        if (u < 0 || v < 0 || u >= cols || v >= rows) return null;
        int px = (int) Math.min(gfxW - 1, Math.floor(u / cols * gfxW));
        int py = (int) Math.min(gfxH - 1, Math.floor(v / rows * gfxH));
        return new int[] {px, py};
    }
}
//?}
