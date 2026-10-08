package com.example.evanscomputermod.radio.medium;

import java.util.function.Consumer;

/**
 * The view of a dimension the path model traces through. The in-world adapter
 * reads loaded chunks (never loading any) and falls back to chunk RF summaries;
 * unit tests implement it over a synthetic grid. Only called from the thread
 * that recomputes links (the server thread), never from {@code transmit}.
 */
public interface RfWorld {

    /** Sentinel for an unknown surface height (chunk neither loaded nor summarised). */
    int UNKNOWN = Integer.MIN_VALUE;

    /** The block at a position, or null when its chunk is unknown (treated as air). */
    RfBlock block(int x, int y, int z);

    /** First y above the MOTION_BLOCKING surface of a column, or {@link #UNKNOWN}. */
    int surfaceY(int x, int z);

    /** Lowest valid block y (bottom of the world). */
    default int minY() {
        return -64;
    }

    /**
     * Every other block volume (Sable sub-level) whose world bounds meet the box.
     * Volumes are visited in their own local block space.
     */
    default void volumes(double minX, double minY, double minZ, double maxX, double maxY, double maxZ,
                         Consumer<RfVolume> sink) {
    }

    /** A block volume with its own frame (a Sable sub-level): local blocks plus a world→local transform. */
    interface RfVolume {
        /** Blocks in local (plot) coordinates. */
        RfWorld blocks();

        /** World position → local position, written to {@code out}. */
        void toLocal(double x, double y, double z, double[] out);
    }
}
