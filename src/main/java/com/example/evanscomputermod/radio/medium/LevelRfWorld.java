package com.example.evanscomputermod.radio.medium;

//? if <=1.21.1 {
import com.example.evanscomputermod.sable.SableCompat;
import net.minecraft.core.BlockPos;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.chunk.LevelChunk;
import net.minecraft.world.level.levelgen.Heightmap;

import java.util.LinkedHashMap;
import java.util.Map;
import java.util.function.Consumer;

/**
 * {@link RfWorld} over a server level. Reads only chunks that are already loaded
 * ({@code getChunkNow}, never loads); unloaded chunks fall back to their
 * {@link Summary} (column heights + the dominant material under the surface),
 * kept from when they unloaded. Sable sub-levels are offered as volumes.
 * Server thread only.
 */
public final class LevelRfWorld implements RfWorld {

    /** RF summary of one unloaded chunk: per column the surface height and the material under it. */
    public static final class Summary {
        final short[] heights = new short[256];
        final RfBlock[] ground = new RfBlock[256];

        static Summary of(LevelChunk chunk) {
            Summary s = new Summary();
            BlockPos.MutableBlockPos p = new BlockPos.MutableBlockPos();
            int bx = chunk.getPos().getMinBlockX(), bz = chunk.getPos().getMinBlockZ();
            int minY = chunk.getMinBuildHeight();
            for (int lx = 0; lx < 16; lx++)
                for (int lz = 0; lz < 16; lz++) {
                    int top = chunk.getHeight(Heightmap.Types.MOTION_BLOCKING, lx, lz) + 1;
                    int i = lx * 16 + lz;
                    s.heights[i] = (short) top;
                    // Dominant (highest-loss) material in the 8 blocks under the surface.
                    RfBlock best = RfBlock.AIR;
                    double bestDb = -1;
                    for (int y = top - 1; y >= Math.max(minY, top - 8); y--) {
                        RfBlock b = RfAttenuation.of(chunk.getBlockState(p.set(bx + lx, y, bz + lz)));
                        double db = b.lossDb(1, 1e9);
                        if (db > bestDb) {
                            bestDb = db;
                            best = b;
                        }
                    }
                    s.ground[i] = best;
                }
            return s;
        }
    }

    /** Summaries of one dimension, oldest evicted past {@link #MAX_SUMMARIES}. */
    public static final class Summaries {
        public static final int MAX_SUMMARIES = 32768;
        private final Map<Long, Summary> map = new LinkedHashMap<>(1024, 0.75f, true) {
            @Override protected boolean removeEldestEntry(Map.Entry<Long, Summary> e) { return size() > MAX_SUMMARIES; }
        };

        public void put(LevelChunk chunk) {
            map.put(chunk.getPos().toLong(), Summary.of(chunk));
        }

        public void remove(long chunkPos) {
            map.remove(chunkPos);
        }

        Summary get(int cx, int cz) {
            return map.get(net.minecraft.world.level.ChunkPos.asLong(cx, cz));
        }

        public int size() {
            return map.size();
        }
    }

    private final ServerLevel level;
    private final Summaries summaries;
    private final BlockPos.MutableBlockPos pos = new BlockPos.MutableBlockPos();
    private int cachedCx = Integer.MIN_VALUE, cachedCz;
    private LevelChunk cachedChunk;

    public LevelRfWorld(ServerLevel level, Summaries summaries) {
        this.level = level;
        this.summaries = summaries;
    }

    private LevelChunk chunk(int cx, int cz) {
        if (cx != cachedCx || cz != cachedCz) {
            cachedCx = cx;
            cachedCz = cz;
            cachedChunk = level.getChunkSource().getChunkNow(cx, cz);
        }
        return cachedChunk;
    }

    @Override
    public RfBlock block(int x, int y, int z) {
        if (y < level.getMinBuildHeight() || y >= level.getMaxBuildHeight()) return RfBlock.AIR;
        LevelChunk c = chunk(x >> 4, z >> 4);
        if (c == null) {
            Summary s = summaries.get(x >> 4, z >> 4);
            if (s == null) return null;
            int i = (x & 15) * 16 + (z & 15);
            return y < s.heights[i] ? s.ground[i] : RfBlock.AIR;
        }
        return RfAttenuation.of(c.getBlockState(pos.set(x, y, z)));
    }

    @Override
    public int surfaceY(int x, int z) {
        LevelChunk c = chunk(x >> 4, z >> 4);
        if (c == null) {
            Summary s = summaries.get(x >> 4, z >> 4);
            return s == null ? UNKNOWN : s.heights[(x & 15) * 16 + (z & 15)];
        }
        return c.getHeight(Heightmap.Types.MOTION_BLOCKING, x & 15, z & 15) + 1;
    }

    @Override
    public int minY() {
        return level.getMinBuildHeight();
    }

    @Override
    public void volumes(double minX, double minY, double minZ, double maxX, double maxY, double maxZ, Consumer<RfVolume> sink) {
        if (SableCompat.isLoaded()) SableRf.volumes(level, minX, minY, minZ, maxX, maxY, maxZ, sink);
    }

    public ServerLevel level() {
        return level;
    }
}
//?}
