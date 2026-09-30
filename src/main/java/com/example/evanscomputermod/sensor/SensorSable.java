package com.example.evanscomputermod.sensor;

//? if <=1.21.1 {
import com.example.evanscomputermod.sable.SableCompat;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.Vec3;
import org.jetbrains.annotations.Nullable;

import java.util.Objects;

/**
 * Sable (sub-level / Create Aeronautics structure) helpers for wires and
 * sensors. Without Sable every position is already a world position. The
 * Sable classes are only touched from {@link Impl}, which loads once Sable
 * is known to be present.
 */
public final class SensorSable {
    private SensorSable() {
    }

    /** Plot-to-world transform of the structure a point is on (identity off-structure). */
    public interface Frame {
        Frame IDENTITY = new Frame() {
            @Override
            public Vec3 position(Vec3 local) {
                return local;
            }

            @Override
            public Vec3 normal(Vec3 local) {
                return local;
            }

            @Override
            public boolean onStructure() {
                return false;
            }
        };

        Vec3 position(Vec3 local);

        Vec3 normal(Vec3 local);

        boolean onStructure();
    }

    /** Frame of the structure {@code pos} is on. */
    public static Frame frame(Level level, Vec3 pos) {
        if(!SableCompat.isLoaded())
            return Frame.IDENTITY;
        return Impl.frame(level, pos);
    }

    public static boolean sameSubLevel(Level level, Vec3 a, Vec3 b) {
        if(!SableCompat.isLoaded())
            return true;
        return Objects.equals(Impl.id(level, a), Impl.id(level, b));
    }

    public static boolean inPlot(Level level, Vec3 pos) {
        return SableCompat.isLoaded() && Impl.id(level, pos) != null;
    }

    /** World position of a point given in a structure's plot space (unchanged elsewhere). */
    public static Vec3 toWorld(Level level, Vec3 pos) {
        if(!SableCompat.isLoaded())
            return pos;
        return Impl.toWorld(level, pos);
    }

    /** A world-space point in the coordinates of {@code entity}'s structure (unchanged off-structure). */
    public static Vec3 toEntityLocal(Entity entity, Vec3 world) {
        if(!SableCompat.isLoaded())
            return world;
        return Impl.toEntityLocal(entity, world);
    }

    /** Client: render orientation of the structure {@code pos} is on, or null off-structure. */
    @Nullable
    public static org.joml.Quaternionf clientOrientation(Vec3 pos, float partialTick) {
        if(!SableCompat.isLoaded())
            return null;
        return Impl.clientOrientation(pos, partialTick);
    }

    private static final class Impl {
        @Nullable
        static org.joml.Quaternionf clientOrientation(Vec3 pos, float partialTick) {
            var sub = dev.ryanhcode.sable.companion.SableCompanion.INSTANCE.getContainingClient(pos);
            return sub == null ? null : new org.joml.Quaternionf(sub.renderPose(partialTick).orientation());
        }

        static Frame frame(Level level, Vec3 pos) {
            var sub = dev.ryanhcode.sable.companion.SableCompanion.INSTANCE.getContaining(level, pos);
            if(sub == null)
                return Frame.IDENTITY;
            var pose = sub.logicalPose();
            return new Frame() {
                @Override
                public Vec3 position(Vec3 local) {
                    return pose.transformPosition(local);
                }

                @Override
                public Vec3 normal(Vec3 local) {
                    return pose.transformNormal(local);
                }

                @Override
                public boolean onStructure() {
                    return true;
                }
            };
        }

        @Nullable
        static java.util.UUID id(Level level, Vec3 pos) {
            var sub = dev.ryanhcode.sable.companion.SableCompanion.INSTANCE.getContaining(level, pos);
            return sub == null ? null : sub.getUniqueId();
        }

        static Vec3 toWorld(Level level, Vec3 pos) {
            return dev.ryanhcode.sable.companion.SableCompanion.INSTANCE.projectOutOfSubLevel(level, pos);
        }

        static Vec3 toEntityLocal(Entity entity, Vec3 world) {
            var sub = dev.ryanhcode.sable.companion.SableCompanion.INSTANCE.getContaining(entity);
            return sub == null ? world : sub.logicalPose().transformPositionInverse(world);
        }
    }
}
//?}
