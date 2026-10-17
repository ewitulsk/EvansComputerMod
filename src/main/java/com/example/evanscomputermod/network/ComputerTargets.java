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

    /**
     * Handle a display packet from {@code player} (ack or input) on the
     * network thread (DISPLAY_PIPELINE_SPEC.md Part A3): when the host keeps
     * a {@link com.example.evanscomputermod.computer.pipeline.ViewerSnapshot},
     * the player is checked against it (the range rule at the last tick) and
     * {@code fast} runs right here; a host without one gets {@code slow} on
     * the server thread, as before. Packets from players the snapshot refuses
     * are dropped.
     */
    public static void offThread(ServerPlayer player, UUID computerId, int displayId, boolean input,
                                 java.util.function.Consumer<IComputerHost> fast, Runnable slow) {
        IComputerHost host = ComputerRegistry.get(computerId);
        var snap = host == null ? null : host.viewerSnapshot();
        if (snap == null) {
            var server = player.level().getServer();
            if (server != null) server.execute(slow);
            return;
        }
        boolean ok = input ? snap.mayInput(player.getUUID(), displayId) : snap.mayInteract(player.getUUID(), displayId);
        if (ok) fast.accept(host);
    }

    /**
     * Input from {@code player} to a computer display, handled on the network
     * thread when the host allows it (Part A3): the compositor and the kernel
     * input queue are thread-safe. The upload port's "who uses this computer"
     * note still happens on the server thread, only when the player changes.
     * {@code slow} is the server-thread path for hosts without a snapshot.
     */
    public static void input(ServerPlayer player, UUID computerId, int displayId,
                             java.util.function.Consumer<ComputerInstance> action, Runnable slow) {
        offThread(player, computerId, displayId, true, host -> {
            ComputerInstance computer = instance(host);
            if (computer == null) return;
            if (!computer.uploadPort().isOperator(player, host)) {
                var server = host.getServer();
                if (server != null) server.execute(() -> computer.uploadPort().noteOperator(player, host));
            }
            action.accept(computer);
            host.markDirtyLater();
        }, slow);
    }

    /** The live computer behind {@code host}, or null while it is off or booting. */
    @Nullable
    public static ComputerInstance instance(IComputerHost host) {
        if (host.getServer() == null) return null;
        return ComputerHost.get(host.getServer(), host.getComputerId()).instance();
    }

    private ComputerTargets() {}
}
