package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.BlockPos;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.ResourceLocation;

/**
 * Packet sent from server to client to update the terminal display.
 * Contains the raw framebuffer data (header + cells) for synchronization.
 */
public record TerminalOutputPacket(
        BlockPos pos,
        byte[] framebufferData
) implements CustomPacketPayload {

    public static final Type<TerminalOutputPacket> TYPE =
            new Type<>(ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "terminal_output"));

    public static final StreamCodec<ByteBuf, TerminalOutputPacket> STREAM_CODEC = StreamCodec.composite(
            BlockPos.STREAM_CODEC, TerminalOutputPacket::pos,
            ByteBufCodecs.BYTE_ARRAY, TerminalOutputPacket::framebufferData,
            TerminalOutputPacket::new
    );

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    /**
     * Creates a packet from a terminal block entity.
     */
    public static TerminalOutputPacket fromBlockEntity(TerminalBlockEntity te) {
        return new TerminalOutputPacket(
                te.getBlockPos(),
                te.getDisplay().toBytes()
        );
    }
}
