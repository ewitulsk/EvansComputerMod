package com.example.evanscomputermod.radio.compat.sable;

//? if <=1.21.1 {
import com.example.evanscomputermod.sable.SableCompat;
import net.minecraft.core.BlockPos;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.event.tick.ServerTickEvent;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Ships for the radio release scenarios: a Sable sub-level built from world
 * blocks that a scenario can fly (hold at a pose every tick, so gravity and
 * collisions don't move it between steps), turn, bring back and set down
 * again. Sable classes are only touched from {@link Impl}, after
 * {@link SableCompat#isLoaded()}, so this class loads without Sable.
 *
 * <p>Flying is kinematic on purpose: the release gate is about what radio does
 * while a ship moves, so the scenario puts the ship exactly where it wants
 * it (Sable's pipeline teleport) instead of steering a physics body.
 */
public final class SableShips {

    private SableShips() {}

    /** A ship assembled by {@link #assemble} (or found by {@link #at}); identity is the sub-level's UUID. */
    public static final class Ship {
        final Object sub;
        final ServerLevel level;
        /** World block of the anchor when it was assembled; plot positions are {@code worldBuildPos + offset}. */
        final BlockPos anchor;
        final BlockPos offset;

        Ship(Object sub, ServerLevel level, BlockPos anchor, BlockPos offset) {
            this.sub = sub;
            this.level = level;
            this.anchor = anchor;
            this.offset = offset;
        }

        public UUID id() { return Impl.id(this); }
        public boolean removed() { return Impl.removed(this); }
        public ServerLevel level() { return level; }
        /** Where a block that was at {@code builtAt} (world, before assembly) lives now in the plot. */
        public BlockPos plotPos(BlockPos builtAt) { return builtAt.offset(offset); }
        /** The world block a plot block came from (inverse of {@link #plotPos}). */
        public BlockPos builtPos(BlockPos plot) { return plot.subtract(offset); }
        /** World position of a plot-space point. */
        public Vec3 toWorld(Vec3 plot) { return Impl.toWorld(this, plot); }
        /** Ship centre (logical pose position) in the world. */
        public Vec3 position() { return Impl.position(this); }
        /** Orientation as {x, y, z, w}. */
        public double[] orientation() { return Impl.orientation(this); }
        /** World-space image of a ship-local direction. */
        public Vec3 rotate(Vec3 local) { return Impl.rotate(this, local); }

        @Override
        public String toString() {
            return "Ship[" + id() + " at " + position() + "]";
        }
    }

    // ------------------------------------------------------------ assembly

    /**
     * Assemble {@code blocks} (world positions) into a new sub-level centred on {@code anchor}.
     * Returns null without Sable.
     */
    public static Ship assemble(ServerLevel level, BlockPos anchor, List<BlockPos> blocks, BlockPos min, BlockPos max) {
        if (!SableCompat.isLoaded()) return null;
        return Impl.assemble(level, anchor, blocks, min, max);
    }

    /** Wrap an existing sub-level whose blocks moved by {@code offset} (plot = world + offset); {@code anchor} is ours. */
    public static Ship wrap(ServerLevel level, Object subLevel, BlockPos anchor, BlockPos offset) {
        return new Ship(subLevel, level, anchor, offset);
    }

    /** The Sable sub-level object (for callers that hand it to another mod's API). */
    public static Object subLevel(Ship ship) {
        return ship.sub;
    }

    /** The world block the ship was built around. */
    public static BlockPos anchor(Ship ship) {
        return ship.anchor;
    }

    /**
     * Put every block of the ship back into the world so that the block built at
     * {@code builtAnchor} lands on {@code goal} (no rotation), the way Sable
     * moves blocks in (block entities keep their NBT, our move hooks run).
     * Returns the number of blocks moved.
     */
    public static int disassemble(Ship ship, BlockPos goal) {
        release(ship);
        return Impl.disassemble(ship, goal);
    }

    /** Remove the ship and its blocks outright (scenario clean-up). */
    public static void discard(Ship ship) {
        release(ship);
        Impl.discard(ship);
    }

    // ------------------------------------------------------------ flying

    private record Hold(Ship ship, double x, double y, double z, double qx, double qy, double qz, double qw) {}

    private static final Map<UUID, Hold> HELD = new ConcurrentHashMap<>();
    private static volatile boolean listening;

    /**
     * Move the ship to world position (x, y, z) with orientation {@code q} = {x, y, z, w} now,
     * and keep it there every server tick until {@link #release} (or the next hold).
     */
    public static void hold(Ship ship, double x, double y, double z, double[] q) {
        listen();
        Hold h = new Hold(ship, x, y, z, q[0], q[1], q[2], q[3]);
        HELD.put(ship.id(), h);
        Impl.place(h);
    }

    /** Hold where it is now, with its current orientation turned by {@code degrees} about world axis (ax, ay, az). */
    public static void turn(Ship ship, double ax, double ay, double az, double degrees) {
        Vec3 p = ship.position();
        double[] q = Impl.turned(ship.orientation(), ax, ay, az, Math.toRadians(degrees));
        hold(ship, p.x, p.y, p.z, q);
    }

    public static void release(Ship ship) {
        if (ship != null) HELD.remove(Impl.id(ship));
    }

    /** True while {@code ship} is held at a pose. */
    public static boolean held(Ship ship) {
        return ship != null && HELD.containsKey(Impl.id(ship));
    }

    private static synchronized void listen() {
        if (listening) return;
        listening = true;
        NeoForge.EVENT_BUS.addListener((ServerTickEvent.Pre e) -> {
            if (HELD.isEmpty()) return;
            List<UUID> gone = new ArrayList<>();
            for (var en : HELD.entrySet()) {
                if (en.getValue().ship().removed()) gone.add(en.getKey());
                else Impl.place(en.getValue());
            }
            gone.forEach(HELD::remove);
        });
    }

    /** Force-load (or release) the chunks of a straight run of world x at {@code z} (ships flying out of loaded land). */
    public static void forceChunks(ServerLevel level, int x0, int x1, int z, boolean on) {
        for (int cx = Math.min(x0, x1) >> 4; cx <= Math.max(x0, x1) >> 4; cx++) level.setChunkForced(cx, z >> 4, on);
    }

    /** A ChunkPos helper for callers that only want to know what {@link #forceChunks} touches. */
    public static List<ChunkPos> chunksOf(int x0, int x1, int z) {
        List<ChunkPos> out = new ArrayList<>();
        for (int cx = Math.min(x0, x1) >> 4; cx <= Math.max(x0, x1) >> 4; cx++) out.add(new ChunkPos(cx, z >> 4));
        return out;
    }

    // ------------------------------------------------------------ Sable

    private static final class Impl {
        static dev.ryanhcode.sable.sublevel.ServerSubLevel sub(Ship s) {
            return (dev.ryanhcode.sable.sublevel.ServerSubLevel) s.sub;
        }

        static Ship assemble(ServerLevel level, BlockPos anchor, List<BlockPos> blocks, BlockPos min, BlockPos max) {
            var sub = dev.ryanhcode.sable.api.SubLevelAssemblyHelper.assembleBlocks(level, anchor, blocks,
                    new dev.ryanhcode.sable.companion.math.BoundingBox3i(min, max));
            return sub == null ? null : new Ship(sub, level, anchor, sub.getPlot().getCenterBlock().subtract(anchor));
        }

        static UUID id(Ship s) { return sub(s).getUniqueId(); }

        static boolean removed(Ship s) { return sub(s).isRemoved(); }

        static Vec3 toWorld(Ship s, Vec3 plot) {
            var v = sub(s).logicalPose().transformPosition(new org.joml.Vector3d(plot.x, plot.y, plot.z));
            return new Vec3(v.x, v.y, v.z);
        }

        static Vec3 position(Ship s) {
            var p = sub(s).logicalPose().position();
            return new Vec3(p.x(), p.y(), p.z());
        }

        static double[] orientation(Ship s) {
            var q = sub(s).logicalPose().orientation();
            return new double[] {q.x(), q.y(), q.z(), q.w()};
        }

        static Vec3 rotate(Ship s, Vec3 local) {
            var v = sub(s).logicalPose().orientation().transform(new org.joml.Vector3d(local.x, local.y, local.z));
            return new Vec3(v.x, v.y, v.z);
        }

        static double[] turned(double[] q, double ax, double ay, double az, double rad) {
            var r = new org.joml.Quaterniond().fromAxisAngleRad(ax, ay, az, rad).mul(new org.joml.Quaterniond(q[0], q[1], q[2], q[3]));
            r.normalize();
            return new double[] {r.x, r.y, r.z, r.w};
        }

        static void place(Hold h) {
            var sub = sub(h.ship());
            if (sub.isRemoved()) return;
            var pos = new org.joml.Vector3d(h.x(), h.y(), h.z());
            var q = new org.joml.Quaterniond(h.qx(), h.qy(), h.qz(), h.qw());
            var handle = dev.ryanhcode.sable.api.physics.handle.RigidBodyHandle.of(sub);
            if (handle == null || !handle.isValid()) {
                sub.logicalPose().position().set(pos);
                sub.logicalPose().orientation().set(q);
                return;
            }
            handle.teleport(pos, q);
            // Cancel what gravity and contacts gave it since the last tick.
            var lin = handle.getLinearVelocity(new org.joml.Vector3d()).negate();
            var ang = handle.getAngularVelocity(new org.joml.Vector3d()).negate();
            handle.addLinearAndAngularVelocity(lin, ang);
            // The logical pose follows the body on the next physics step; set it now so this tick's readers see the move.
            sub.logicalPose().position().set(pos);
            sub.logicalPose().orientation().set(q);
        }

        static int disassemble(Ship s, BlockPos goal) {
            var sub = sub(s);
            if (sub.isRemoved()) return 0;
            ServerLevel level = s.level;
            List<BlockPos> blocks = plotBlocks(s);
            BlockPos plotAnchor = s.plotPos(s.anchor);
            var transform = new dev.ryanhcode.sable.api.SubLevelAssemblyHelper.AssemblyTransform(plotAnchor, goal, 0,
                    net.minecraft.world.level.block.Rotation.NONE, level);
            if (!blocks.isEmpty()) dev.ryanhcode.sable.api.SubLevelAssemblyHelper.moveBlocks(level, transform, blocks);
            return blocks.size();
        }

        static List<BlockPos> plotBlocks(Ship s) {
            var sub = sub(s);
            var box = sub.getPlot().getBoundingBox();
            List<BlockPos> blocks = new ArrayList<>();
            for (BlockPos p : BlockPos.betweenClosed(box.minX(), box.minY(), box.minZ(), box.maxX(), box.maxY(), box.maxZ()))
                if (!s.level.getBlockState(p).isAir()) blocks.add(p.immutable());
            return blocks;
        }

        static void discard(Ship s) {
            var sub = sub(s);
            if (sub.isRemoved()) return;
            var container = dev.ryanhcode.sable.api.sublevel.SubLevelContainer.getContainer(s.level);
            if (container != null)
                container.removeSubLevel(sub, dev.ryanhcode.sable.sublevel.storage.SubLevelRemovalReason.REMOVED);
        }
    }
}
//?}
