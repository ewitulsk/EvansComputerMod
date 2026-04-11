package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.BlockPos;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerPlayer;
import net.neoforged.neoforge.network.handling.IPayloadContext;

/**
 * Client → Server: acknowledges receipt of a delta/keyframe and signals
 * readiness for the next update. Implements VNC-style client-paced updates.
 * <p>
 * {@code target} is {@link TerminalDeltaPacket#TARGET_TERMINAL} for the
 * main terminal GUI display, or {@link TerminalDeltaPacket#TARGET_SCREEN}
 * for the in-world Screen cluster display. The server routes the ack to
 * the correct per-target sync-state map.
 */
public record TerminalReadyPacket(
        BlockPos pos,
        byte target,
        long lastGeneration
) implements CustomPacketPayload {

    public static final Type<TerminalReadyPacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "terminal_ready"));

    public static final StreamCodec<ByteBuf, TerminalReadyPacket> STREAM_CODEC = StreamCodec.composite(
            BlockPos.STREAM_CODEC, TerminalReadyPacket::pos,
            ByteBufCodecs.BYTE, TerminalReadyPacket::target,
            ByteBufCodecs.VAR_LONG, TerminalReadyPacket::lastGeneration,
            TerminalReadyPacket::new
    );

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    public static void handle(TerminalReadyPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            if (context.player() instanceof ServerPlayer player) {
                if (player.level().getBlockEntity(packet.pos()) instanceof TerminalBlockEntity te) {
                    te.onClientReady(player.getUUID(), packet.target(), packet.lastGeneration());
                }
            }
        });
    }
}
