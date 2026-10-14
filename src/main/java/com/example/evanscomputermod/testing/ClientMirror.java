package com.example.evanscomputermod.testing;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.DisplaySyncHub;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.network.DeltaApplier;
import com.example.evanscomputermod.network.TerminalDeltaPacket;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;

import java.util.ArrayList;
import java.util.List;
import java.util.UUID;

/**
 * Plays one client watching a terminal, headless, using the real code on
 * both ends:
 * <ul>
 *   <li>{@link #pump} is the delta protocol: a
 *       {@link DisplaySyncHub} streaming the terminal's display syncs this
 *       client as its viewer, and each packet is serialised and parsed like
 *       the real one, applied with the client's {@link DeltaApplier}, then
 *       acked. The hub is the mirror's own, so the terminal's real sync
 *       passes (which only keep players with the GUI open) leave it alone.</li>
 *   <li>{@link #blockEntityUpdate} is a vanilla block-entity data packet (what
 *       {@code sendBlockUpdated} and chunk (re)sends deliver): the server's
 *       {@code getUpdateTag}, loaded into the client-side block entity the
 *       way NeoForge's default {@code onDataPacket} does.</li>
 * </ul>
 * The client-side block entity is a detached {@link TerminalBlockEntity},
 * so its {@code loadAdditional} is the real client load path.
 */
public final class ClientMirror {
    private final TerminalBlockEntity server;
    private final TerminalBlockEntity client;
    private final UUID viewer = UUID.randomUUID();
    private final DisplaySyncHub hub = new DisplaySyncHub(IComputerHost.DISPLAY_OWN);

    /** Like a player opening the terminal: fresh sync state, keyframe first. */
    public ClientMirror(TerminalBlockEntity server) {
        this.server = server;
        this.client = new TerminalBlockEntity(server.getBlockPos(), server.getBlockState());
        hub.requestKeyframe(viewer);
    }

    /** Deliver every pending sync packet. Returns how many were applied. */
    public int pump() {
        int n = 0;
        for (int round = 0; round < 16; round++) {
            List<TerminalDeltaPacket> sent = new ArrayList<>();
            hub.sync(server.getComputerId(), server.getDisplay(), List.of(viewer), (id, p) -> sent.add(p));
            if (sent.isEmpty()) break;
            for (TerminalDeltaPacket p : sent) {
                TerminalDeltaPacket.ParsedDelta d = p.parse();
                if (d.packetType() == TerminalDeltaPacket.PACKET_TYPE_KEYFRAME) {
                    DeltaApplier.applyKeyframe(client.getDisplay(), d);
                } else {
                    DeltaApplier.applyDelta(client.getDisplay(), d);
                }
                hub.onClientReady(viewer, d.generation());
                n++;
            }
        }
        return n;
    }

    /** A block-entity data packet arrives. */
    public void blockEntityUpdate(HolderLookup.Provider registries) {
        CompoundTag tag = server.getUpdateTag(registries);
        if (!tag.isEmpty()) client.loadWithComponents(tag, registries);
    }

    /** What this client shows. */
    public TerminalDisplay display() {
        return client.getDisplay();
    }
}
//?}
