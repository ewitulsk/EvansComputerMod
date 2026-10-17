package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.IComputerHost;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.UUIDUtil;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerPlayer;
import net.neoforged.neoforge.network.handling.IPayloadContext;

import java.util.UUID;

/**
 * Client → Server: acknowledges receipt of a delta/keyframe and signals
 * readiness for the next update. Implements VNC-style client-paced updates.
 * <p>
 * Addressed by computer id and display id ({@link IComputerHost#DISPLAY_OWN},
 * {@link IComputerHost#DISPLAY_ATTACHED}); the host routes the ack to that
 * display's sync state. Generation 0 asks for a keyframe.
 */
public record TerminalReadyPacket(
        UUID computerId,
        byte displayId,
        long lastGeneration,
        /** When the client received the acked frame, on its own clock (µs; 0 = unknown). Feeds RTT/jitter (Part B2). */
        long clientRecvUs
) implements CustomPacketPayload {

    public static final Type<TerminalReadyPacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "terminal_ready"));

    public static final StreamCodec<ByteBuf, TerminalReadyPacket> STREAM_CODEC = StreamCodec.composite(
            UUIDUtil.STREAM_CODEC, TerminalReadyPacket::computerId,
            ByteBufCodecs.BYTE, TerminalReadyPacket::displayId,
            ByteBufCodecs.VAR_LONG, TerminalReadyPacket::lastGeneration,
            ByteBufCodecs.VAR_LONG, TerminalReadyPacket::clientRecvUs,
            TerminalReadyPacket::new
    );

    public TerminalReadyPacket(UUID computerId, byte displayId, long lastGeneration) {
        this(computerId, displayId, lastGeneration, 0);
    }

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    /** Network thread (Part A3): straight to the host's sync hub, checked against its viewer snapshot. */
    public static void handle(TerminalReadyPacket packet, IPayloadContext context) {
        if (!(context.player() instanceof ServerPlayer player)) return;
        ComputerTargets.offThread(player, packet.computerId(), packet.displayId(), false,
                host -> host.onDisplayAck(player.getUUID(), packet.displayId(), packet.lastGeneration(), packet.clientRecvUs()),
                () -> {
                    IComputerHost host = ComputerTargets.resolve(player, packet.computerId(), packet.displayId());
                    if (host != null) host.onDisplayAck(player, packet.displayId(), packet.lastGeneration());
                });
    }
}
