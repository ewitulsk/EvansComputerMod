package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.BlockPos;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;

public record LoadProgramResponsePacket(BlockPos pos, String jsonContent) implements CustomPacketPayload {

    public static final Type<LoadProgramResponsePacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "load_program_response"));

    public static final StreamCodec<ByteBuf, LoadProgramResponsePacket> STREAM_CODEC = StreamCodec.composite(
            BlockPos.STREAM_CODEC, LoadProgramResponsePacket::pos,
            ByteBufCodecs.STRING_UTF8, LoadProgramResponsePacket::jsonContent,
            LoadProgramResponsePacket::new
    );

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

}
