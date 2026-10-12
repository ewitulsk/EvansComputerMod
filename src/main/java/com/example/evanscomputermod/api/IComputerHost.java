package com.example.evanscomputermod.api;

import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerPlayer;

import org.jetbrains.annotations.Nullable;
import java.util.UUID;

/**
 * The core contract that any context embedding a ComputerInstance must implement.
 * This is the only interface a third-party mod author MUST implement to embed
 * a computer into their own block, entity, or item.
 *
 * <p>Clients address a computer by {@link #getComputerId()} plus a display id:
 * {@link #DISPLAY_OWN} for the host's own screen, {@link #DISPLAY_ATTACHED}
 * for an attached second display. Ids above that are reserved.
 */
public interface IComputerHost {

    /** Display id of the host's own screen (a Terminal's GUI framebuffer). */
    int DISPLAY_OWN = 0;
    /** Display id of an attached second display (a Terminal's Screen cluster). */
    int DISPLAY_ATTACHED = 1;

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
    default IVisualProgramming getVisualProgramming() {
        return null;
    }

    /** Peripherals attached to this computer, or null if the host has none (programs then see an empty list). */
    @Nullable
    default com.example.evanscomputermod.computer.peripheral.PeripheralHub getPeripheralHub() {
        return null;
    }

    // ---- Displays and input (all optional) ----

    /** The attached second display ({@link #DISPLAY_ATTACHED}), or null when none is attached. */
    @Nullable
    default IAttachedDisplay getAttachedDisplay() {
        return null;
    }

    /**
     * The display panel tier of display {@code displayId}: what it can show
     * and every limit computed from it (text grid, graphics size, refresh,
     * colours, touch). Null when the display has no panel (the host's own
     * display runs headless) or there is no such display. Default: Tier 2
     * for both displays, the limits from before panels existed.
     */
    @Nullable
    default com.example.evanscomputermod.computer.display.PanelTier displayTier(int displayId) {
        if (displayId == DISPLAY_OWN) return com.example.evanscomputermod.computer.display.PanelTier.COLOR_LCD;
        if (displayId == DISPLAY_ATTACHED && getAttachedDisplay() != null) {
            return getAttachedDisplay().tier();
        }
        return null;
    }

    /** Phosphor colour of a Tier 1 display (ignored for other tiers). */
    default com.example.evanscomputermod.computer.display.PanelTier.Tone displayTone(int displayId) {
        return com.example.evanscomputermod.computer.display.PanelTier.Tone.GREEN;
    }

    /**
     * Whether the host's own display takes pointer input. Mouse capture
     * ({@code mouse_capture_start}) only succeeds on hosts that do, and only
     * while the display shows graphics.
     */
    default boolean acceptsPointerInput() {
        return false;
    }

    /**
     * Whether {@code player} may watch and send input to display
     * {@code displayId}: the range and permission check behind every
     * client-to-server display packet. Server thread.
     */
    default boolean canInteract(ServerPlayer player, int displayId) {
        return false;
    }

    /**
     * Whether {@code player} may send keyboard and pointer input to display
     * {@code displayId} (what {@code TerminalInputPacket} and
     * {@code MouseInputPacket} check). Default: {@link #canInteract}. A host
     * that streams a display to onlookers who may watch but not type (the
     * ePhone's mirror for nearby players) narrows it. Server thread.
     */
    default boolean acceptsInput(ServerPlayer player, int displayId) {
        return canInteract(player, displayId);
    }

    /**
     * Whether the host's own display is a portrait (ePhone) display: program
     * framebuffer limits and {@code gfx_modes}' text grid and largest size
     * then come from the panel tier's portrait mode instead of the
     * Terminal's landscape one. Default: false.
     */
    default boolean portraitDisplay() {
        return false;
    }

    /**
     * A client acknowledged display frames up to {@code generation} on
     * {@code displayId}, or asked for a keyframe ({@code generation == 0}).
     * Server thread.
     */
    default void onDisplayAck(ServerPlayer player, int displayId, long generation) {}

    /** The kernel switched network interface {@code index}'s link (block hosts show it on their faces). */
    default void onLinkStateChanged(int index, boolean up) {}

    /** Wireless controllers connected to this computer, or null if the host takes none. */
    @Nullable
    default com.example.evanscomputermod.controller.WirelessControllerHub getControllers() {
        return null;
    }
}
