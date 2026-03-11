package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.client.VisualProgrammingScreen;
import io.netty.buffer.ByteBuf;
import net.minecraft.client.Minecraft;
import net.minecraft.core.BlockPos;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.ResourceLocation;
import net.neoforged.neoforge.network.handling.IPayloadContext;

/**
 * Packet sent from server to client to open the visual programming editor.
 */
public record OpenVisualEditorPacket(BlockPos pos) implements CustomPacketPayload {

    public static final Type<OpenVisualEditorPacket> TYPE =
            new Type<>(ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "open_visual_editor"));

    public static final StreamCodec<ByteBuf, OpenVisualEditorPacket> STREAM_CODEC = StreamCodec.composite(
            BlockPos.STREAM_CODEC, OpenVisualEditorPacket::pos,
            OpenVisualEditorPacket::new
    );

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    public static void handle(OpenVisualEditorPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            Minecraft mc = Minecraft.getInstance();
            mc.setScreen(new VisualProgrammingScreen(packet.pos()));
        });
    }
}
