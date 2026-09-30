package com.example.evanscomputermod.worldgen;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.computer.WorldNetwork;

import net.minecraft.core.BlockPos;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.levelgen.Heightmap;
import net.neoforged.neoforge.event.level.ChunkEvent;

/** Materializes only the chord fragments in a loaded chunk. Missing chunks stay intact. */
public final class FiberWorld {
    private static final java.util.Queue<Runnable> pending =
            new java.util.concurrent.ConcurrentLinkedQueue<>();

    public static void serverTick(net.neoforged.neoforge.event.tick.ServerTickEvent.Post event) {
        // Chunk-load callbacks run before FULL promotion finishes. Placement there can
        // deadlock physics mods that inspect adjacent chunks during a block change.
        int count = Math.min(8, pending.size());
        for (int i = 0; i < count; i++) {
            Runnable job = pending.poll();
            if (job != null) job.run();
        }
    }
    public static String linkAt(WorldNetwork d, BlockPos p) {
        for (int i = 0; i < d.villages.size(); i++) {
            var a = d.villages.get(i);
            var b = d.villages.get((i + 1) % d.villages.size());
            double dx = b.x() - a.x(), dz = b.z() - a.z(), length = dx * dx + dz * dz;
            double t = ((p.getX() - a.x()) * dx + (p.getZ() - a.z()) * dz) / length;
            if (t < 0 || t > 1) continue;
            if (Math.hypot(p.getX() - a.x() - t * dx, p.getZ() - a.z() - t * dz) < 3)
                return WorldNetwork.linkName(a.number(), b.number());
        }
        return null;
    }

    public static void chunkLoaded(ChunkEvent.Load event) {
        if (!(event.getLevel() instanceof ServerLevel level)
                || level.dimension() != Level.OVERWORLD) return;
        var pos = event.getChunk().getPos();
        for (int dx = -1; dx <= 1; dx++)
            for (int dz = -1; dz <= 1; dz++)
                queue(level, new net.minecraft.world.level.ChunkPos(pos.x + dx, pos.z + dz));
    }

    private static void queue(ServerLevel level, net.minecraft.world.level.ChunkPos pos) {
        pending.add(
                        () -> {
                            if (!level.hasChunk(pos.x, pos.z)) return;
                            var d = WorldNetwork.get(level);
                            if (d.villages.isEmpty()) return;
                            java.util.Set<Long> columns = new java.util.HashSet<>();
                            for (long known : d.generatedFiber) {
                                BlockPos p = BlockPos.of(known);
                                if ((p.getX() >> 4) == pos.x && (p.getZ() >> 4) == pos.z)
                                    columns.add(new BlockPos(p.getX(), 0, p.getZ()).asLong());
                            }
                            for (int i = 0; i < d.villages.size(); i++) {
                                var a = d.villages.get(i);
                                var b = d.villages.get((i + 1) % 10);
                                double distance = Math.hypot(b.x() - a.x(), b.z() - a.z());
                                int count = (int) Math.ceil(distance);
                                if (Math.max(a.x(), b.x()) < pos.getMinBlockX()
                                        || Math.min(a.x(), b.x()) > pos.getMaxBlockX()
                                        || Math.max(a.z(), b.z()) < pos.getMinBlockZ()
                                        || Math.min(a.z(), b.z()) > pos.getMaxBlockZ()) continue;
                                for (int step = 0; step <= count; step++) {
                                    int
                                            x =
                                                    (int)
                                                            Math.round(
                                                                    a.x()
                                                                            + (b.x() - a.x())
                                                                                    * (double) step
                                                                                    / count),
                                            z =
                                                    (int)
                                                            Math.round(
                                                                    a.z()
                                                                            + (b.z() - a.z())
                                                                                    * (double) step
                                                                                    / count);
                                    if ((x >> 4) != pos.x || (z >> 4) != pos.z) continue;
                                    // Neighbor updates and physics must never pull in terrain.
                                    if (!level.hasChunk((x - 16) >> 4, (z - 16) >> 4)
                                            || !level.hasChunk((x + 16) >> 4, (z - 16) >> 4)
                                            || !level.hasChunk((x - 16) >> 4, (z + 16) >> 4)
                                            || !level.hasChunk((x + 16) >> 4, (z + 16) >> 4)) continue;
                                    if (!columns.add(new BlockPos(x, 0, z).asLong())) continue;
                                    int floor = level.getHeight(Heightmap.Types.OCEAN_FLOOR, x, z);
                                    int surface =
                                            level.getHeight(Heightmap.Types.WORLD_SURFACE, x, z);
                                    boolean ocean = surface > floor + 2;
                                    int y = ocean ? floor : surface + 5;
                                    BlockPos span = new BlockPos(x, y, z);
                                    if (!d.brokenFiber.contains(span.asLong())
                                            && (level.getBlockState(span).isAir()
                                                    || level.getBlockState(span)
                                                            .getFluidState()
                                                            .isSource())) {
                                        level.setBlock(
                                                span,
                                                ModBlocks.FIBER_SPAN.get().defaultBlockState(),
                                                3);
                                        d.generatedFiber.add(span.asLong());
                                        d.setDirty();
                                    }
                                    if (!ocean && step % 16 == 0)
                                        for (int dy = surface; dy < y; dy++) {
                                            BlockPos pole = new BlockPos(x, dy, z);
                                            if (!d.brokenFiber.contains(pole.asLong())
                                                    && level.getBlockState(pole).isAir()) {
                                                level.setBlock(
                                                        pole,
                                                        ModBlocks.UTILITY_POLE
                                                                .get()
                                                                .defaultBlockState(),
                                                        3);
                                                d.generatedFiber.add(pole.asLong());
                                                d.setDirty();
                                            }
                                        }
                                }
                            }
                        });
    }
}
//?}
