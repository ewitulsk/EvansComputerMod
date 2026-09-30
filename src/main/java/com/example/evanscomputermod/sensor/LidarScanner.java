package com.example.evanscomputermod.sensor;

//? if <=1.21.1 {
import com.example.evanscomputermod.sensor.wire.BaseWireEntity;
import net.minecraft.core.BlockPos;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.level.ClipContext;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.HitResult;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.CollisionContext;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * Casts lidar rays. Rays run through the world with vanilla block clipping,
 * which Sable extends to every structure (including the one the sensor is
 * on), and are also tested against entities. A server-wide budget of
 * {@link #RAYS_PER_TICK} rays per tick spreads big scans over several ticks,
 * like a spinning lidar does.
 */
public final class LidarScanner {
    public static final int RAYS_PER_TICK = 4096;
    /** Ray length between chunk-loaded checks: rays never load chunks, they stop at unloaded ones. */
    private static final double CHUNK_STEP = 8;

    private static long budgetTick = Long.MIN_VALUE;
    private static int budgetUsed;

    private LidarScanner() {
    }

    /** Claim up to {@code want} rays from this tick's budget. */
    static synchronized int claim(MinecraftServer server, int want) {
        long tick = server.getTickCount();
        if(tick != budgetTick) {
            budgetTick = tick;
            budgetUsed = 0;
        }
        int granted = Math.max(0, Math.min(want, RAYS_PER_TICK - budgetUsed));
        budgetUsed += granted;
        return granted;
    }

    /** A scan in progress. */
    static final class Job {
        final LidarConfig config;
        final SensorMount mount;
        final float[] ranges;
        final long startTick;
        int next;

        Job(LidarConfig config, SensorMount mount, long startTick) {
            this.config = config;
            this.mount = mount;
            this.ranges = new float[config.rays()];
            this.startTick = startTick;
        }

        boolean done() {
            return next >= ranges.length;
        }
    }

    /** Cast up to {@code maxRays} more rays of {@code job}. */
    static void advance(ServerLevel level, Job job, int maxRays) {
        if(maxRays <= 0 || job.done())
            return;
        SensorSable.Frame frame = SensorSable.frame(level, job.mount.origin());
        Vec3 from = frame.position(job.mount.origin());
        double range = job.config.range();
        List<Entity> entities = level.getEntities((Entity) null, new AABB(from, from).inflate(range), LidarScanner::blocksRays);
        int end = Math.min(job.ranges.length, job.next + maxRays);
        for(int i = job.next; i < end; i++) {
            double[] d = job.config.direction(i);
            Vec3 dir = frame.normal(job.mount.sensorToBlock(d[0], d[1], d[2])).normalize();
            job.ranges[i] = (float) cast(level, from, dir, range, entities);
        }
        job.next = end;
    }

    static boolean blocksRays(Entity e) {
        return !(e instanceof BaseWireEntity) && !e.isSpectator() && e.isAlive()
                && (e instanceof LivingEntity || e.isPickable());
    }

    /** Distance to the first hit along {@code dir} within {@code range}, or +infinity. */
    static double cast(ServerLevel level, Vec3 from, Vec3 dir, double range, List<Entity> entities) {
        // Stop at the first unloaded chunk instead of loading it.
        double reach = 0;
        while(reach < range) {
            double step = Math.min(range, reach + CHUNK_STEP);
            if(!level.hasChunkAt(BlockPos.containing(from.add(dir.scale(step)))))
                break;
            reach = step;
        }
        if(reach <= 0)
            return Double.POSITIVE_INFINITY;
        Vec3 to = from.add(dir.scale(reach));
        double best = Double.POSITIVE_INFINITY;
        var hit = level.clip(new ClipContext(from, to, ClipContext.Block.COLLIDER, ClipContext.Fluid.NONE, CollisionContext.empty()));
        if(hit.getType() != HitResult.Type.MISS) {
            Vec3 at = SensorSable.toWorld(level, hit.getLocation());
            best = at.distanceTo(from);
        }
        for(Entity e : entities) {
            var box = e.getBoundingBox();
            if(box.contains(from))
                continue;
            var h = box.clip(from, to);
            if(h.isPresent())
                best = Math.min(best, h.get().distanceTo(from));
        }
        return best <= range ? best : Double.POSITIVE_INFINITY;
    }

    /** A finished scan. */
    public record Scan(int seq, LidarConfig config, SensorMount mount, long startTick, long endTick, float[] ranges) {

        /** Ranges as little-endian float32, row-major (row = elevation, column = azimuth); misses are +inf. */
        public byte[] rangeBytes() {
            ByteBuffer buf = ByteBuffer.allocate(ranges.length * 4).order(ByteOrder.LITTLE_ENDIAN);
            for(float r : ranges)
                buf.putFloat(r);
            return buf.array();
        }

        /** Hit points in the computer frame (x forward, y left, z up), little-endian float32 triples. */
        public byte[] pointBytes() {
            int hits = 0;
            for(float r : ranges)
                if(Float.isFinite(r)) hits++;
            ByteBuffer buf = ByteBuffer.allocate(hits * 12).order(ByteOrder.LITTLE_ENDIAN);
            Vec3 origin = mount.offset();
            for(int i = 0; i < ranges.length; i++) {
                float r = ranges[i];
                if(!Float.isFinite(r))
                    continue;
                double[] d = config.direction(i);
                Vec3 p = origin.add(mount.blockToComputer(mount.sensorToBlock(d[0], d[1], d[2])).scale(r));
                buf.putFloat((float) p.x).putFloat((float) p.y).putFloat((float) p.z);
            }
            return buf.array();
        }

        public int hits() {
            return (int) java.util.stream.IntStream.range(0, ranges.length).filter(i -> Float.isFinite(ranges[i])).count();
        }

        public Map<String, Object> toMap(boolean ranges, boolean points) {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("seq", seq);
            m.put("start_tick", startTick);
            m.put("end_tick", endTick);
            m.put("rows", config.rows());
            m.put("columns", config.azSteps());
            m.put("config", config.toMap());
            m.put("hits", hits());
            if(ranges)
                m.put("ranges", rangeBytes());
            if(points)
                m.put("points", pointBytes());
            return m;
        }
    }
}
//?}
