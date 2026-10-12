package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.UUIDUtil;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;

import java.util.UUID;

/**
 * Packet sent from server to client to open the visual programming editor
 * for a computer.
 */
public record OpenVisualEditorPacket(UUID computerId) implements CustomPacketPayload {

    public static final Type<OpenVisualEditorPacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "open_visual_editor"));

    public static final StreamCodec<ByteBuf, OpenVisualEditorPacket> STREAM_CODEC = StreamCodec.composite(
            UUIDUtil.STREAM_CODEC, OpenVisualEditorPacket::computerId,
            OpenVisualEditorPacket::new
    );

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

}
