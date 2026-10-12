package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.computer.ComputerInstance;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.UUIDUtil;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerPlayer;
import net.neoforged.neoforge.network.handling.IPayloadContext;

import java.util.UUID;

/**
 * Client → server: a pointer event on one of a computer's displays, in that
 * display's framebuffer pixels.
 */
public record MouseInputPacket(
        UUID computerId,
        byte displayId,
        byte kind,
        short x,
        short y,
        byte buttons,
        byte buttonCode,
        byte scrollDir
) implements CustomPacketPayload {

    public static final byte KIND_MOVE = 1;
    public static final byte KIND_DOWN = 2;
    public static final byte KIND_UP = 3;
    public static final byte KIND_SCROLL = 4;

    public static final Type<MouseInputPacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "mouse_input"));

    public static final StreamCodec<ByteBuf, MouseInputPacket> STREAM_CODEC = new StreamCodec<>() {
        @Override
        public MouseInputPacket decode(ByteBuf buf) {
            UUID id = UUIDUtil.STREAM_CODEC.decode(buf);
            byte displayId = buf.readByte();
            byte kind = buf.readByte();
            short x = buf.readShort();
            short y = buf.readShort();
            byte buttons = buf.readByte();
            byte buttonCode = buf.readByte();
            byte scrollDir = buf.readByte();
            return new MouseInputPacket(id, displayId, kind, x, y, buttons, buttonCode, scrollDir);
        }

        @Override
        public void encode(ByteBuf buf, MouseInputPacket packet) {
            UUIDUtil.STREAM_CODEC.encode(buf, packet.computerId());
            buf.writeByte(packet.displayId());
            buf.writeByte(packet.kind());
            buf.writeShort(packet.x());
            buf.writeShort(packet.y());
            buf.writeByte(packet.buttons());
            buf.writeByte(packet.buttonCode());
            buf.writeByte(packet.scrollDir());
        }
    };

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    public static void handle(MouseInputPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            try {
                if (!(context.player() instanceof ServerPlayer player)) return;
                IComputerHost host = ComputerTargets.resolve(player, packet.computerId(), packet.displayId());
                if (host == null) return;
                ComputerInstance computer = ComputerTargets.instance(host);
                if (computer != null) {
                    computer.handlePointerInput(packet.displayId(), packet.kind(), packet.x(), packet.y(),
                            packet.buttons(), packet.buttonCode(), packet.scrollDir());
                }
            } catch (Throwable e) {
                EvansComputerMod.LOGGER.error("Error handling mouse input packet", e);
            }
        });
    }
}
