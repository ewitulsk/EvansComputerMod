package com.example.evanscomputermod.testing;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.ClientSyncState;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.network.DeltaApplier;
import com.example.evanscomputermod.network.TerminalDeltaPacket;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;

/**
 * Plays one client watching a terminal, headless, using the real code on
 * both ends:
 * <ul>
 *   <li>{@link #pump} is the delta protocol: the server's
 *       {@link TerminalBlockEntity#nextSyncPacket} for this client's sync
 *       state, serialised and parsed like the real packet, applied with the
 *       client's {@link DeltaApplier}, then acked.</li>
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
    private final ClientSyncState state = new ClientSyncState();

    /** Like a player opening the terminal: fresh sync state, keyframe first. */
    public ClientMirror(TerminalBlockEntity server) {
        this.server = server;
        this.client = new TerminalBlockEntity(server.getBlockPos(), server.getBlockState());
        state.init(server.getDisplay());
        state.needsKeyframe = true;
    }

    /** Deliver every pending sync packet. Returns how many were applied. */
    public int pump() {
        int n = 0;
        while (n < 16) {
            if (!state.shouldSendKeyframe() && !state.isClientReady()) break;
            TerminalDeltaPacket p = server.nextSyncPacket(state);
            if (p == null) break;
            TerminalDeltaPacket.ParsedDelta d = p.parse();
            if (d.packetType() == TerminalDeltaPacket.PACKET_TYPE_KEYFRAME) {
                DeltaApplier.applyKeyframe(client.getDisplay(), d);
            } else {
                DeltaApplier.applyDelta(client.getDisplay(), d);
            }
            state.onClientReady(d.generation());
            n++;
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
