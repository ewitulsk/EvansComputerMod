package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.computer.VisualPrograms;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.UUIDUtil;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerPlayer;
import net.neoforged.neoforge.network.PacketDistributor;
import net.neoforged.neoforge.network.handling.IPayloadContext;

import java.util.UUID;

/**
 * Client → server: list the computer's saved visual programs; answered with a
 * {@link ProgramListResponsePacket}.
 */
public record RequestProgramListPacket(UUID computerId) implements CustomPacketPayload {

    public static final Type<RequestProgramListPacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "request_program_list"));

    public static final StreamCodec<ByteBuf, RequestProgramListPacket> STREAM_CODEC = StreamCodec.composite(
            UUIDUtil.STREAM_CODEC, RequestProgramListPacket::computerId,
            RequestProgramListPacket::new
    );

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    public static void handle(RequestProgramListPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            if (!(context.player() instanceof ServerPlayer player)) return;
            IComputerHost host = ComputerTargets.resolve(player, packet.computerId(), IComputerHost.DISPLAY_OWN);
            if (host == null) return;
            String fileList = String.join("\n", VisualPrograms.list(host));
            PacketDistributor.sendToPlayer(player, new ProgramListResponsePacket(packet.computerId(), fileList));
        });
    }
}
