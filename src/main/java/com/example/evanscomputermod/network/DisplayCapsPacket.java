package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import io.netty.buffer.ByteBuf;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;
import net.neoforged.neoforge.network.handling.IPayloadContext;

/**
 * Server to client, when a player joins: the server's cap on the client's
 * playout delay for timed frames ({@code display.maxPlayoutMs},
 * DISPLAY_PIPELINE_SPEC.md Part E1).
 */
public record DisplayCapsPacket(int maxPlayoutMs) implements CustomPacketPayload {
    public static final Type<DisplayCapsPacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "display_caps"));

    public static final StreamCodec<ByteBuf, DisplayCapsPacket> STREAM_CODEC = StreamCodec.composite(
            ByteBufCodecs.VAR_INT, DisplayCapsPacket::maxPlayoutMs, DisplayCapsPacket::new);

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    public static void handle(DisplayCapsPacket p, IPayloadContext ctx) {
        ClientPlayout.maxPlayoutMs = Math.max(60, Math.min(400, p.maxPlayoutMs()));
    }
}
