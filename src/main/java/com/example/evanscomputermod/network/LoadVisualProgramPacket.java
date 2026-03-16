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
import net.minecraft.server.level.ServerPlayer;
import net.neoforged.neoforge.network.PacketDistributor;
import net.neoforged.neoforge.network.handling.IPayloadContext;

public record LoadVisualProgramPacket(BlockPos pos, String fileName) implements CustomPacketPayload {

    public static final Type<LoadVisualProgramPacket> TYPE =
            new Type<>(ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "load_visual_program"));

    public static final StreamCodec<ByteBuf, LoadVisualProgramPacket> STREAM_CODEC = StreamCodec.composite(
            BlockPos.STREAM_CODEC, LoadVisualProgramPacket::pos,
            ByteBufCodecs.STRING_UTF8, LoadVisualProgramPacket::fileName,
            LoadVisualProgramPacket::new
    );

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    public static void handle(LoadVisualProgramPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            var player = context.player();
            if (player.level() instanceof ServerLevel serverLevel) {
                var blockEntity = serverLevel.getBlockEntity(packet.pos());
                if (blockEntity instanceof TerminalBlockEntity terminal) {
                    String json = terminal.loadVisualProgram(packet.fileName());
                    if (json != null && player instanceof ServerPlayer serverPlayer) {
                        PacketDistributor.sendToPlayer(serverPlayer,
                                new LoadProgramResponsePacket(packet.pos(), json));
                    }
                }
            }
        });
    }
}
