package com.example.evanscomputermod.radio.medium;

//? if <=1.21.1 {
import com.example.evanscomputermod.sable.SableCompat;
import net.minecraft.core.BlockPos;
import net.minecraft.server.level.ServerLevel;

import java.util.HashMap;
import java.util.HashSet;
import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.function.Consumer;

/**
 * Sable sub-levels for the radio medium (spec 3E/5D): sub-levels as traceable
 * {@link RfWorld.RfVolume}s (blocks read in plot space, rays moved into it with
 * the inverse logical pose), the sub-level a world point is on (per-ship
 * recompute rate limit), and sub-level motion → region invalidation. Sable
 * classes are only touched from {@link Impl}, after {@link SableCompat#isLoaded()}.
 */
public final class SableRf {

    private SableRf() {}

    static void volumes(ServerLevel level, double minX, double minY, double minZ, double maxX, double maxY, double maxZ,
                        Consumer<RfWorld.RfVolume> sink) {
        if (SableCompat.isLoaded()) Impl.volumes(level, minX, minY, minZ, maxX, maxY, maxZ, sink);
    }

    /** UUID of the sub-level whose world bounds contain the point, or null. */
    static UUID subLevelAt(ServerLevel level, double x, double y, double z) {
        return SableCompat.isLoaded() ? Impl.subLevelAt(level, x, y, z) : null;
    }

    /** Tracks sub-level poses of one level and reports moved regions past the thresholds. */
    public static final class Tracker {
        private final Map<UUID, double[]> last = new HashMap<>();   // x, y, z, qx, qy, qz, qw, minX..maxZ, tick

        /** Calls {@code moved} with the union of old and new world bounds for each sub-level that moved past the thresholds. */
        public void tick(ServerLevel level, long tick, double metres, double radians, int minTicks, RegionSink moved) {
            if (!SableCompat.isLoaded()) return;
            Impl.track(level, tick, metres, radians, minTicks, last, moved);
        }
    }

    public interface RegionSink {
        void changed(double minX, double minY, double minZ, double maxX, double maxY, double maxZ);
    }

    private static final class Impl {
        static void volumes(ServerLevel level, double minX, double minY, double minZ, double maxX, double maxY, double maxZ,
                            Consumer<RfWorld.RfVolume> sink) {
            var box = new dev.ryanhcode.sable.companion.math.BoundingBox3d(minX, minY, minZ, maxX, maxY, maxZ);
            var container = dev.ryanhcode.sable.api.sublevel.SubLevelContainer.getContainer(level);
            if (container == null) return;
            for (var sub : dev.ryanhcode.sable.companion.SableCompanion.INSTANCE.getAllIntersecting(level, box)) {
                var pose = sub.logicalPose();
                RfWorld blocks = new PlotWorld(level, container);
                sink.accept(new RfWorld.RfVolume() {
                    final org.joml.Vector3d v = new org.joml.Vector3d();

                    @Override public RfWorld blocks() { return blocks; }

                    @Override public void toLocal(double x, double y, double z, double[] out) {
                        v.set(x, y, z);
                        pose.transformPositionInverse(v);
                        out[0] = v.x;
                        out[1] = v.y;
                        out[2] = v.z;
                    }
                });
            }
        }

        static UUID subLevelAt(ServerLevel level, double x, double y, double z) {
            var box = new dev.ryanhcode.sable.companion.math.BoundingBox3d(x - 0.25, y - 0.25, z - 0.25, x + 0.25, y + 0.25, z + 0.25);
            for (var sub : dev.ryanhcode.sable.companion.SableCompanion.INSTANCE.getAllIntersecting(level, box))
                return sub.getUniqueId();
            return null;
        }

        static void track(ServerLevel level, long tick, double metres, double radians, int minTicks,
                          Map<UUID, double[]> last, RegionSink moved) {
            var container = dev.ryanhcode.sable.api.sublevel.SubLevelContainer.getContainer(level);
            if (container == null) return;
            Set<UUID> seen = new HashSet<>();
            for (var sub : container.getAllSubLevels()) {
                UUID id = sub.getUniqueId();
                seen.add(id);
                var pose = sub.logicalPose();
                var p = pose.position();
                var q = pose.orientation();
                var bb = sub.boundingBox();
                double[] now = {p.x(), p.y(), p.z(), q.x(), q.y(), q.z(), q.w(),
                        bb.minX(), bb.minY(), bb.minZ(), bb.maxX(), bb.maxY(), bb.maxZ(), tick};
                double[] was = last.get(id);
                if (was == null) {
                    last.put(id, now);
                    moved.changed(now[7], now[8], now[9], now[10], now[11], now[12]);
                    continue;
                }
                double dx = now[0] - was[0], dy = now[1] - was[1], dz = now[2] - was[2];
                double dot = Math.abs(now[3] * was[3] + now[4] * was[4] + now[5] * was[5] + now[6] * was[6]);
                double box = 0;
                for (int i = 7; i <= 12; i++) box = Math.max(box, Math.abs(now[i] - was[i]));
                boolean past = Math.sqrt(dx * dx + dy * dy + dz * dz) > metres || 2 * Math.acos(Math.min(1, dot)) > radians
                        || box > metres;   // bounds settle a tick after assembly; blocks added/removed grow them
                if (past && tick - (long) was[13] >= minTicks) {
                    moved.changed(Math.min(was[7], now[7]), Math.min(was[8], now[8]), Math.min(was[9], now[9]),
                            Math.max(was[10], now[10]), Math.max(was[11], now[11]), Math.max(was[12], now[12]));
                    last.put(id, now);
                }
            }
            last.keySet().removeIf(id -> {
                if (seen.contains(id)) return false;
                double[] was = last.get(id);
                moved.changed(was[7], was[8], was[9], was[10], was[11], was[12]);
                return true;
            });
        }
    }

    /** Blocks in Sable's plot grid (plot chunks are not in the level's normal chunk map). */
    private static final class PlotWorld implements RfWorld {
        private final ServerLevel level;
        private final dev.ryanhcode.sable.api.sublevel.SubLevelContainer container;
        private final BlockPos.MutableBlockPos pos = new BlockPos.MutableBlockPos();

        PlotWorld(ServerLevel level, dev.ryanhcode.sable.api.sublevel.SubLevelContainer container) {
            this.level = level;
            this.container = container;
        }

        @Override
        public RfBlock block(int x, int y, int z) {
            if (y < level.getMinBuildHeight() || y >= level.getMaxBuildHeight()) return RfBlock.AIR;
            var chunk = container.getChunk(new net.minecraft.world.level.ChunkPos(x >> 4, z >> 4));
            if (chunk == null) return RfBlock.AIR;
            return RfAttenuation.of(chunk.getBlockState(pos.set(x, y, z)));
        }

        @Override
        public int surfaceY(int x, int z) {
            return UNKNOWN;
        }
    }
}
//?}
