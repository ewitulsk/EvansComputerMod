package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.computer.VisualPrograms;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.UUIDUtil;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerPlayer;
import net.neoforged.neoforge.network.handling.IPayloadContext;

import java.util.UUID;

/**
 * Client → server: save a visual program into the computer's storage.
 */
public record SaveVisualProgramPacket(UUID computerId, String fileName, String jsonContent) implements CustomPacketPayload {

    public static final Type<SaveVisualProgramPacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "save_visual_program"));

    public static final StreamCodec<ByteBuf, SaveVisualProgramPacket> STREAM_CODEC = StreamCodec.composite(
            UUIDUtil.STREAM_CODEC, SaveVisualProgramPacket::computerId,
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
            if (!(context.player() instanceof ServerPlayer player)) return;
            IComputerHost host = ComputerTargets.resolve(player, packet.computerId(), IComputerHost.DISPLAY_OWN);
            if (host != null) {
                VisualPrograms.save(host, packet.fileName(), packet.jsonContent());
            }
        });
    }
}
