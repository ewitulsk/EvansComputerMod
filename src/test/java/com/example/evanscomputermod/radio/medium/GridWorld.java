package com.example.evanscomputermod.radio.medium;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.function.Consumer;

/** Synthetic {@link RfWorld}: solid ground below {@code groundY}, placed blocks above, optional volumes. Counts reads. */
final class GridWorld implements RfWorld {
    final int groundY;
    final RfBlock ground;
    final Map<Long, RfBlock> blocks = new HashMap<>();
    final Map<Long, Integer> tops = new HashMap<>();
    final List<RfVolume> volumes = new ArrayList<>();
    /** Unknown (unloaded, unsummarised) region: x >= unknownFromX. */
    int unknownFromX = Integer.MAX_VALUE;
    long reads;

    GridWorld(int groundY, RfBlock ground) {
        this.groundY = groundY;
        this.ground = ground;
    }

    static long key(int x, int y, int z) {
        return ((long) x & 0x3FFFFFF) << 38 | ((long) z & 0x3FFFFFF) << 12 | (y & 0xFFF);
    }

    static long col(int x, int z) {
        return ((long) x << 32) ^ (z & 0xFFFFFFFFL);
    }

    GridWorld set(int x, int y, int z, RfBlock b) {
        blocks.put(key(x, y, z), b);
        if (!b.isAir()) tops.merge(col(x, z), y + 1, Math::max);
        return this;
    }

    GridWorld fill(int x0, int y0, int z0, int x1, int y1, int z1, RfBlock b) {
        for (int x = Math.min(x0, x1); x <= Math.max(x0, x1); x++)
            for (int y = Math.min(y0, y1); y <= Math.max(y0, y1); y++)
                for (int z = Math.min(z0, z1); z <= Math.max(z0, z1); z++) set(x, y, z, b);
        return this;
    }

    /** A hollow box (walls, floor and roof) of {@code b}. */
    GridWorld shell(int x0, int y0, int z0, int x1, int y1, int z1, RfBlock b) {
        for (int x = x0; x <= x1; x++)
            for (int y = y0; y <= y1; y++)
                for (int z = z0; z <= z1; z++)
                    if (x == x0 || x == x1 || y == y0 || y == y1 || z == z0 || z == z1) set(x, y, z, b);
        return this;
    }

    @Override
    public RfBlock block(int x, int y, int z) {
        reads++;
        if (x >= unknownFromX) return null;
        RfBlock b = blocks.get(key(x, y, z));
        if (b != null) return b;
        return y < groundY ? ground : RfBlock.AIR;
    }

    @Override
    public int surfaceY(int x, int z) {
        reads++;
        if (x >= unknownFromX) return UNKNOWN;
        return Math.max(groundY, tops.getOrDefault(col(x, z), groundY));
    }

    @Override
    public void volumes(double minX, double minY, double minZ, double maxX, double maxY, double maxZ, Consumer<RfVolume> sink) {
        volumes.forEach(sink);
    }

    /** A volume: its own grid, placed in the world by translation (ox, oy, oz) and a yaw rotation. */
    static RfVolume volume(GridWorld local, double ox, double oy, double oz, double yawRad) {
        double c = Math.cos(yawRad), s = Math.sin(yawRad);
        return new RfVolume() {
            @Override public RfWorld blocks() { return local; }
            @Override public void toLocal(double x, double y, double z, double[] out) {
                double dx = x - ox, dz = z - oz;
                out[0] = c * dx + s * dz;
                out[1] = y - oy;
                out[2] = -s * dx + c * dz;
            }
        };
    }
}
