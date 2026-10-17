package com.example.evanscomputermod.network;

import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.computer.ComputerHost;
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.ComputerRegistry;
import net.minecraft.server.level.ServerPlayer;

import org.jetbrains.annotations.Nullable;
import java.util.UUID;

/**
 * Server side of client-to-server display packets: which host a packet's
 * computer id names, and whether the sending player may use it. Packets
 * name a computer by UUID plus a display id; the host decides range and
 * permission ({@link IComputerHost#canInteract}), so a host that isn't a
 * block works the same way.
 */
public final class ComputerTargets {

    /** The host running {@code computerId} if {@code player} may use display {@code displayId}, else null. */
    @Nullable
    public static IComputerHost resolve(ServerPlayer player, UUID computerId, int displayId) {
        IComputerHost host = ComputerRegistry.get(computerId);
        if (host == null || host.getServer() != player.level().getServer()) return null;
        return host.canInteract(player, displayId) ? host : null;
    }

    /**
     * The host running {@code computerId} if {@code player} may send input to
     * display {@code displayId} ({@link IComputerHost#acceptsInput}), else null.
     */
    @Nullable
    public static IComputerHost resolveInput(ServerPlayer player, UUID computerId, int displayId) {
        IComputerHost host = resolve(player, computerId, displayId);
        if (host == null || !host.acceptsInput(player, displayId)) return null;
        // Whoever types or clicks is the player an upload asks for a file (UploadPort).
        ComputerInstance computer = instance(host);
        if (computer != null) computer.uploadPort().noteOperator(player, host);
        return host;
    }

    /** The live computer behind {@code host}, or null while it is off or booting. */
    @Nullable
    public static ComputerInstance instance(IComputerHost host) {
        if (host.getServer() == null) return null;
        return ComputerHost.get(host.getServer(), host.getComputerId()).instance();
    }

    private ComputerTargets() {}
}
