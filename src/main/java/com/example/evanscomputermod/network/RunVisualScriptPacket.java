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

/**
 * Packet sent from client to server to run a visual program on a terminal.
 * Contains the generated Python code and the terminal's block position.
 */
public record RunVisualScriptPacket(BlockPos pos, String pythonCode) implements CustomPacketPayload {

    public static final Type<RunVisualScriptPacket> TYPE =
            new Type<>(ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "run_visual_script"));

    public static final StreamCodec<ByteBuf, RunVisualScriptPacket> STREAM_CODEC = StreamCodec.composite(
            BlockPos.STREAM_CODEC, RunVisualScriptPacket::pos,
            ByteBufCodecs.STRING_UTF8, RunVisualScriptPacket::pythonCode,
            RunVisualScriptPacket::new
    );

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    public static void handle(RunVisualScriptPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            var player = context.player();
            if (player.level() instanceof ServerLevel serverLevel) {
                var blockEntity = serverLevel.getBlockEntity(packet.pos());
                if (blockEntity instanceof TerminalBlockEntity terminal) {
                    terminal.runVisualScript(packet.pythonCode());
                }
            }
        });
    }
}
