package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.ComputerRegistry;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.BlockPos;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.level.Level;
import net.neoforged.neoforge.network.handling.IPayloadContext;

import java.util.Optional;
import java.util.UUID;

public record MouseInputPacket(
        BlockPos pos,
        byte kind,
        short x,
        short y,
        byte buttons,
        byte buttonCode,
        byte scrollDir,
        Optional<UUID> computerId
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
            BlockPos pos = BlockPos.STREAM_CODEC.decode(buf);
            byte kind = buf.readByte();
            short x = buf.readShort();
            short y = buf.readShort();
            byte buttons = buf.readByte();
            byte buttonCode = buf.readByte();
            byte scrollDir = buf.readByte();
            boolean hasUuid = buf.readBoolean();
            Optional<UUID> computerId = hasUuid
                    ? Optional.of(new UUID(buf.readLong(), buf.readLong()))
                    : Optional.empty();
            return new MouseInputPacket(pos, kind, x, y, buttons, buttonCode, scrollDir, computerId);
        }

        @Override
        public void encode(ByteBuf buf, MouseInputPacket packet) {
            BlockPos.STREAM_CODEC.encode(buf, packet.pos());
            buf.writeByte(packet.kind());
            buf.writeShort(packet.x());
            buf.writeShort(packet.y());
            buf.writeByte(packet.buttons());
            buf.writeByte(packet.buttonCode());
            buf.writeByte(packet.scrollDir());
            buf.writeBoolean(packet.computerId().isPresent());
            packet.computerId().ifPresent(uuid -> {
                buf.writeLong(uuid.getMostSignificantBits());
                buf.writeLong(uuid.getLeastSignificantBits());
            });
        }
    };

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    public static void handle(MouseInputPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            try {
                if (context.player() instanceof ServerPlayer player) {
                    Level level = player.level();
                    BlockPos pos = packet.pos();

                    if (player.distanceToSqr(pos.getX() + 0.5, pos.getY() + 0.5, pos.getZ() + 0.5) > 64) {
                        return;
                    }

                    TerminalBlockEntity te = null;
                    if (packet.computerId().isPresent()) {
                        IComputerHost host = ComputerRegistry.get(packet.computerId().get());
                        if (host instanceof TerminalBlockEntity tbe) {
                            te = tbe;
                        }
                    }
                    if (te == null) {
                        if (level.getBlockEntity(pos) instanceof TerminalBlockEntity tbe) {
                            te = tbe;
                        }
                    }

                    if (te != null) {
                        te.onMouseEvent(packet);
                    }
                }
            } catch (Throwable e) {
                EvansComputerMod.LOGGER.error("Error handling mouse input packet", e);
            }
        });
    }
}
