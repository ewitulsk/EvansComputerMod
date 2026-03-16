package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.client.VisualProgrammingScreen;
import io.netty.buffer.ByteBuf;
import net.minecraft.client.Minecraft;
import net.minecraft.core.BlockPos;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.ResourceLocation;
import net.neoforged.neoforge.network.handling.IPayloadContext;

public record LoadProgramResponsePacket(BlockPos pos, String jsonContent) implements CustomPacketPayload {

    public static final Type<LoadProgramResponsePacket> TYPE =
            new Type<>(ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "load_program_response"));

    public static final StreamCodec<ByteBuf, LoadProgramResponsePacket> STREAM_CODEC = StreamCodec.composite(
            BlockPos.STREAM_CODEC, LoadProgramResponsePacket::pos,
            ByteBufCodecs.STRING_UTF8, LoadProgramResponsePacket::jsonContent,
            LoadProgramResponsePacket::new
    );

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    public static void handle(LoadProgramResponsePacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            Minecraft mc = Minecraft.getInstance();
            if (mc.screen instanceof VisualProgrammingScreen vps) {
                vps.onProgramLoaded(packet.jsonContent());
            }
        });
    }
}
