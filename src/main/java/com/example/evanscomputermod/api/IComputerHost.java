package com.example.evanscomputermod.api;

import net.minecraft.server.MinecraftServer;

import org.jetbrains.annotations.Nullable;
import java.util.UUID;

/**
 * The core contract that any context embedding a ComputerInstance must implement.
 * This is the only interface a third-party mod author MUST implement to embed
 * a computer into their own block, entity, or item.
 */
public interface IComputerHost {

    /** Persistent identity used for file storage and computer lookup. */
    UUID getComputerId();

    /** Access to the Minecraft server for scheduling work on the main thread. */
    @Nullable
    MinecraftServer getServer();

    /** Called when the computer's state changes and needs to be persisted. */
    void markDirty();

    /** Called from the worker thread when terminal output has changed and needs sync to clients. */
    void syncToClients();

    /** Force the next syncToClients to send a full keyframe (bypasses client ack check). */
    default void forceNextKeyframe() {}

    /** Returns the framebuffer display capability, or null if this host has no display (headless). */
    @Nullable
    IFramebufferDisplay getFramebufferDisplay();

    /** Returns the redstone capability, or null if this host cannot do redstone. */
    @Nullable
    IRedstoneProvider getRedstoneProvider();

    /** Returns the world access capability, or null if not positioned in a world. */
    @Nullable
    IWorldAccess getWorldAccess();

    /** Returns the visual programming capability, or null if unsupported. */
    @Nullable
    IVisualProgramming getVisualProgramming();
}
