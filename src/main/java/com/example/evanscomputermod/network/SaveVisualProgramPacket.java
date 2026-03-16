package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.BlockPos;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.level.ServerLevel;
import net.neoforged.neoforge.network.handling.IPayloadContext;

public record SaveVisualProgramPacket(BlockPos pos, String fileName, String jsonContent) implements CustomPacketPayload {

    public static final Type<SaveVisualProgramPacket> TYPE =
            new Type<>(ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "save_visual_program"));

    public static final StreamCodec<ByteBuf, SaveVisualProgramPacket> STREAM_CODEC = StreamCodec.composite(
            BlockPos.STREAM_CODEC, SaveVisualProgramPacket::pos,
            ByteBufCodecs.STRING_UTF8, SaveVisualProgramPacket::fileName,
            ByteBufCodecs.STRING_UTF8, SaveVisualProgramPacket::jsonContent,
            SaveVisualProgramPacket::new
    );

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    public static void handle(SaveVisualProgramPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            var player = context.player();
            if (player.level() instanceof ServerLevel serverLevel) {
                var blockEntity = serverLevel.getBlockEntity(packet.pos());
                if (blockEntity instanceof TerminalBlockEntity terminal) {
                    terminal.saveVisualProgram(packet.fileName(), packet.jsonContent());
                }
            }
        });
    }
}
