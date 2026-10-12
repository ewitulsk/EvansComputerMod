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
 * Packet sent from client to server to run a visual program on a computer.
 * Contains the generated Python code.
 */
public record RunVisualScriptPacket(UUID computerId, String pythonCode) implements CustomPacketPayload {

    public static final Type<RunVisualScriptPacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "run_visual_script"));

    public static final StreamCodec<ByteBuf, RunVisualScriptPacket> STREAM_CODEC = StreamCodec.composite(
            UUIDUtil.STREAM_CODEC, RunVisualScriptPacket::computerId,
            ByteBufCodecs.STRING_UTF8, RunVisualScriptPacket::pythonCode,
            RunVisualScriptPacket::new
    );

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    public static void handle(RunVisualScriptPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            if (!(context.player() instanceof ServerPlayer player)) return;
            IComputerHost host = ComputerTargets.resolve(player, packet.computerId(), IComputerHost.DISPLAY_OWN);
            if (host != null) {
                VisualPrograms.run(host, ComputerTargets.instance(host), packet.pythonCode());
            }
        });
    }
}
