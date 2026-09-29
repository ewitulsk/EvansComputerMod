package com.example.evanscomputermod.compat.create;

//? if <=1.21.1 {
import com.example.evanscomputermod.sable.SableCompat;
import com.simibubi.create.infrastructure.config.AllConfigs;
import net.minecraft.core.BlockPos;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.phys.Vec3;

/**
 * Create Redstone Link range checks, measured the way Create does when Sable
 * is installed (Sable's RedstoneLinkNetworkHandlerMixin): block centres
 * projected out of sub-level plots into world space, compared against the
 * server's {@code linkRange}. Sable only patches the checks inside
 * {@code updateNetworkOf}; this covers the ones we make ourselves.
 */
final class LinkRange {

    private LinkRange() {
    }

    static boolean withinRange(ServerLevel level, BlockPos a, BlockPos b) {
        int range = AllConfigs.server().logistics.linkRange.get();
        if (!SableCompat.isLoaded()) {
            return a.closerThan(b, range);
        }
        Vec3 pa = SableLinks.worldCenter(level, a);
        Vec3 pb = SableLinks.worldCenter(level, b);
        return pa.distanceToSqr(pb) < (double) range * range;
    }

    /** Whether {@code pos} is inside a Sable sub-level (a physics structure that can move). */
    static boolean isOnSubLevel(ServerLevel level, BlockPos pos) {
        return SableCompat.isLoaded() && SableLinks.isOnSubLevel(level, pos);
    }

    /** Sable-touching half, only classloaded when Sable is installed. */
    private static final class SableLinks {
        static Vec3 worldCenter(ServerLevel level, BlockPos pos) {
            org.joml.Vector3d v = dev.ryanhcode.sable.companion.math.JOMLConversion.atCenterOf(pos);
            dev.ryanhcode.sable.sublevel.SubLevel sub = dev.ryanhcode.sable.Sable.HELPER.getContaining(level, v);
            if (sub != null) sub.logicalPose().transformPosition(v);
            return new Vec3(v.x, v.y, v.z);
        }

        static boolean isOnSubLevel(ServerLevel level, BlockPos pos) {
            org.joml.Vector3d v = dev.ryanhcode.sable.companion.math.JOMLConversion.atCenterOf(pos);
            return dev.ryanhcode.sable.Sable.HELPER.getContaining(level, v) != null;
        }
    }
}
//?}
