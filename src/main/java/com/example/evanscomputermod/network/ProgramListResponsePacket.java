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

import java.util.Arrays;
import java.util.List;

public record ProgramListResponsePacket(BlockPos pos, String fileListNewlineSeparated) implements CustomPacketPayload {

    public static final Type<ProgramListResponsePacket> TYPE =
            new Type<>(ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "program_list_response"));

    public static final StreamCodec<ByteBuf, ProgramListResponsePacket> STREAM_CODEC = StreamCodec.composite(
            BlockPos.STREAM_CODEC, ProgramListResponsePacket::pos,
            ByteBufCodecs.STRING_UTF8, ProgramListResponsePacket::fileListNewlineSeparated,
            ProgramListResponsePacket::new
    );

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    public static void handle(ProgramListResponsePacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            Minecraft mc = Minecraft.getInstance();
            if (mc.screen instanceof VisualProgrammingScreen vps) {
                List<String> programs = packet.fileListNewlineSeparated().isEmpty()
                        ? List.of()
                        : Arrays.asList(packet.fileListNewlineSeparated().split("\n"));
                vps.onProgramListReceived(programs);
            }
        });
    }
}
