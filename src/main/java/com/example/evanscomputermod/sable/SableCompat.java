package com.example.evanscomputermod.sable;

//? if <=1.21.1 {
import net.minecraft.core.BlockPos;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.chunk.LevelChunk;
import net.neoforged.fml.ModList;

import dev.ryanhcode.sable.api.sublevel.ServerSubLevelContainer;
import dev.ryanhcode.sable.api.sublevel.SubLevelContainer;
import dev.ryanhcode.sable.sublevel.plot.LevelPlot;

import java.util.List;

/**
 * Runtime feature-flag + routing helpers for the optional sable physics
 * engine integration. Sable is declared as an optional dependency in
 * neoforge.mods.toml for the 1.21.1 build only.
 *
 * <p>This file is guarded out of the 26.1 build; code that calls into it
 * must sit behind a {@code //? if <=1.21.1} directive.
 */
public final class SableCompat {

    private SableCompat() {}

    private static Boolean cached;

    public static boolean isLoaded() {
        Boolean c = cached;
        if (c != null) return c;
        boolean loaded;
        try {
            loaded = ModList.get() != null && ModList.get().isLoaded("sable");
        } catch (Throwable t) {
            loaded = false;
        }
        cached = loaded;
        return loaded;
    }

    /**
     * If {@code pos} falls inside a sable plot in {@code level}, return the
     * list of {@link ServerPlayer}s currently tracking that plot's SubLevel.
     * Returns {@code null} if sable isn't loaded, the position isn't in a
     * plot, or the lookup fails.
     *
     * <p>Plot coordinates are far from players in world-space, so the
     * vanilla {@code ServerLevel.getPlayers(distance-filter)} never matches
     * them. Packet sinks for physics-assembled BEs must use this list
     * instead.
     */
    public static List<ServerPlayer> getPlotTrackingPlayers(ServerLevel level, BlockPos pos) {
        if (!isLoaded()) return null;
        try {
            ServerSubLevelContainer container = SubLevelContainer.getContainer(level);
            if (container == null) return null;
            ChunkPos cp = new ChunkPos(pos);
            if (!container.inBounds(cp)) return null;
            return container.getPlayersTracking(cp);
        } catch (Throwable t) {
            return null;
        }
    }

    /**
     * Client-side counterpart: when a packet carries a plot-space
     * {@code BlockPos}, {@code mc.level.getBlockEntity(pos)} misses because
     * plot chunks aren't in the regular client chunk source. Locate the
     * plot that owns the position and pull the BE out of its plot chunk.
     */
    public static BlockEntity findBlockEntityInSablePlot(Level level, BlockPos pos) {
        if (!isLoaded()) return null;
        try {
            SubLevelContainer container = SubLevelContainer.getContainer(level);
            if (container == null) return null;
            ChunkPos cp = new ChunkPos(pos);
            if (!container.inBounds(cp)) return null;
            LevelPlot plot = container.getPlot(cp);
            if (plot == null) return null;
            LevelChunk chunk = plot.getChunk(cp);
            if (chunk == null) return null;
            return chunk.getBlockEntity(pos);
        } catch (Throwable t) {
            return null;
        }
    }

    /**
     * Resolve a BE at {@code pos} using the regular level lookup first, then
     * falling back to sable's plot chunks if the regular lookup misses.
     * This is the form most callers want.
     */
    public static BlockEntity resolveBlockEntity(Level level, BlockPos pos) {
        BlockEntity be = level.getBlockEntity(pos);
        if (be != null) return be;
        return findBlockEntityInSablePlot(level, pos);
    }
}
//?}
