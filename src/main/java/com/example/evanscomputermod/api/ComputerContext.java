package com.example.evanscomputermod.api;

import net.minecraft.core.BlockPos;
import net.minecraft.server.MinecraftServer;
import net.minecraft.world.level.Level;

import org.jetbrains.annotations.Nullable;
import java.util.UUID;

/**
 * Context object injected as the first parameter of {@link ComputerFunction} methods.
 * Provides access to the computer's environment. Not visible to Python callers.
 */
public class ComputerContext {

    private final UUID computerId;
    @Nullable
    private final BlockPos position;
    @Nullable
    private final Level level;
    @Nullable
    private final MinecraftServer server;

    public ComputerContext(IComputerHost host) {
        this.computerId = host.getComputerId();
        IWorldAccess worldAccess = host.getWorldAccess();
        this.position = worldAccess != null ? worldAccess.getBlockPos() : null;
        this.level = worldAccess != null ? worldAccess.getLevel() : null;
        this.server = host.getServer();
    }

    public UUID getComputerId() { return computerId; }

    @Nullable
    public BlockPos getPosition() { return position; }

    @Nullable
    public Level getLevel() { return level; }

    @Nullable
    public MinecraftServer getServer() { return server; }
}
