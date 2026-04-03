package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.BlockPos;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;

public record ProgramListResponsePacket(BlockPos pos, String fileListNewlineSeparated) implements CustomPacketPayload {

    public static final Type<ProgramListResponsePacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "program_list_response"));

    public static final StreamCodec<ByteBuf, ProgramListResponsePacket> STREAM_CODEC = StreamCodec.composite(
            BlockPos.STREAM_CODEC, ProgramListResponsePacket::pos,
            ByteBufCodecs.STRING_UTF8, ProgramListResponsePacket::fileListNewlineSeparated,
            ProgramListResponsePacket::new
    );

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

}
