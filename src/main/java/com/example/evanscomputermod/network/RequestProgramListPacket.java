package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.BlockPos;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.neoforged.neoforge.network.PacketDistributor;
import net.neoforged.neoforge.network.handling.IPayloadContext;

import java.util.List;

public record RequestProgramListPacket(BlockPos pos) implements CustomPacketPayload {

    public static final Type<RequestProgramListPacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "request_program_list"));

    public static final StreamCodec<ByteBuf, RequestProgramListPacket> STREAM_CODEC = StreamCodec.composite(
            BlockPos.STREAM_CODEC, RequestProgramListPacket::pos,
            RequestProgramListPacket::new
    );

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    public static void handle(RequestProgramListPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            var player = context.player();
            if (player.level() instanceof ServerLevel serverLevel) {
                var blockEntity = serverLevel.getBlockEntity(packet.pos());
                if (blockEntity instanceof TerminalBlockEntity terminal) {
                    List<String> programs = terminal.listVisualPrograms();
                    String fileListJson = String.join("\n", programs);
                    if (player instanceof ServerPlayer serverPlayer) {
                        PacketDistributor.sendToPlayer(serverPlayer,
                                new ProgramListResponsePacket(packet.pos(), fileListJson));
                    }
                }
            }
        });
    }
}
