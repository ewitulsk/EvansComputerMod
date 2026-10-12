package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.UUIDUtil;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;

import java.util.UUID;

public record ProgramListResponsePacket(UUID computerId, String fileListNewlineSeparated) implements CustomPacketPayload {

    public static final Type<ProgramListResponsePacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "program_list_response"));

    public static final StreamCodec<ByteBuf, ProgramListResponsePacket> STREAM_CODEC = StreamCodec.composite(
            UUIDUtil.STREAM_CODEC, ProgramListResponsePacket::computerId,
            ByteBufCodecs.STRING_UTF8, ProgramListResponsePacket::fileListNewlineSeparated,
            ProgramListResponsePacket::new
    );

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

}
