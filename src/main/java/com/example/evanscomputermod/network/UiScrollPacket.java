package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.UUIDUtil;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerPlayer;
import net.neoforged.neoforge.network.handling.IPayloadContext;

import java.util.UUID;

/**
 * Client to server: the player scrolled a scroll container of a GUI the client
 * draws itself, and the client already shows it (DISPLAY_PIPELINE_SPEC.md Part
 * F). {@code surface} is the scene's surface id, {@code node} the app's node
 * id, {@code x, y} the new offset in lu; the app gets {@code Scrolled} and
 * confirms with its next commit. It replaces the wheel event, so the app
 * doesn't scroll twice. Handled on the network thread like other input.
 */
public record UiScrollPacket(UUID computerId, byte displayId, int surface, int node, int x, int y)
        implements CustomPacketPayload {

    public static final Type<UiScrollPacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "ui_scroll"));

    public static final StreamCodec<ByteBuf, UiScrollPacket> STREAM_CODEC = new StreamCodec<>() {
        @Override
        public UiScrollPacket decode(ByteBuf buf) {
            return new UiScrollPacket(UUIDUtil.STREAM_CODEC.decode(buf), buf.readByte(), UiScenePackets.readVar(buf),
                    UiScenePackets.readVar(buf), UiScenePackets.readVar(buf), UiScenePackets.readVar(buf));
        }

        @Override
        public void encode(ByteBuf buf, UiScrollPacket p) {
            UUIDUtil.STREAM_CODEC.encode(buf, p.computerId());
            buf.writeByte(p.displayId());
            UiScenePackets.writeVar(buf, p.surface());
            UiScenePackets.writeVar(buf, p.node());
            UiScenePackets.writeVar(buf, Math.max(0, p.x()));
            UiScenePackets.writeVar(buf, Math.max(0, p.y()));
        }
    };

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    public static void handle(UiScrollPacket packet, IPayloadContext context) {
        if (!(context.player() instanceof ServerPlayer player)) return;
        Runnable none = () -> {};
        ComputerTargets.input(player, packet.computerId(), packet.displayId(),
                c -> c.getUiManager().scrolled(packet.surface(), packet.node(), packet.x(), packet.y()), () -> {
                    var host = ComputerTargets.resolveInput(player, packet.computerId(), packet.displayId());
                    var c = host == null ? null : ComputerTargets.instance(host);
                    if (c != null) c.getUiManager().scrolled(packet.surface(), packet.node(), packet.x(), packet.y());
                    none.run();
                });
    }
}
