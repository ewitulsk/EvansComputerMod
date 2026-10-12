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
import net.neoforged.neoforge.network.PacketDistributor;
import net.neoforged.neoforge.network.handling.IPayloadContext;

import java.util.UUID;

/**
 * Client → server: load a saved visual program; answered with a
 * {@link LoadProgramResponsePacket}.
 */
public record LoadVisualProgramPacket(UUID computerId, String fileName) implements CustomPacketPayload {

    public static final Type<LoadVisualProgramPacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "load_visual_program"));

    public static final StreamCodec<ByteBuf, LoadVisualProgramPacket> STREAM_CODEC = StreamCodec.composite(
            UUIDUtil.STREAM_CODEC, LoadVisualProgramPacket::computerId,
            ByteBufCodecs.STRING_UTF8, LoadVisualProgramPacket::fileName,
            LoadVisualProgramPacket::new
    );

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    public static void handle(LoadVisualProgramPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            if (!(context.player() instanceof ServerPlayer player)) return;
            IComputerHost host = ComputerTargets.resolve(player, packet.computerId(), IComputerHost.DISPLAY_OWN);
            if (host == null) return;
            String json = VisualPrograms.load(host, packet.fileName());
            if (json != null) {
                PacketDistributor.sendToPlayer(player, new LoadProgramResponsePacket(packet.computerId(), json));
            }
        });
    }
}
