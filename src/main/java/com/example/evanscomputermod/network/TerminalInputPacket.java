package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.ComputerRegistry;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.BlockPos;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.level.Level;
import net.neoforged.neoforge.network.handling.IPayloadContext;

import java.util.Optional;
import java.util.UUID;

/**
 * Packet sent from client to server when the player types in the terminal.
 * Supports both single characters and escape sequences for special keys.
 */
public record TerminalInputPacket(
        BlockPos pos,
        String input,
        Optional<UUID> computerId
) implements CustomPacketPayload {

    public static final Type<TerminalInputPacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "terminal_input"));

    public static final StreamCodec<ByteBuf, TerminalInputPacket> STREAM_CODEC = new StreamCodec<>() {
        @Override
        public TerminalInputPacket decode(ByteBuf buf) {
            BlockPos pos = BlockPos.STREAM_CODEC.decode(buf);
            String input = ByteBufCodecs.STRING_UTF8.decode(buf);
            boolean hasUuid = buf.readBoolean();
            Optional<UUID> computerId = hasUuid ? Optional.of(new UUID(buf.readLong(), buf.readLong())) : Optional.empty();
            return new TerminalInputPacket(pos, input, computerId);
        }

        @Override
        public void encode(ByteBuf buf, TerminalInputPacket packet) {
            BlockPos.STREAM_CODEC.encode(buf, packet.pos());
            ByteBufCodecs.STRING_UTF8.encode(buf, packet.input());
            buf.writeBoolean(packet.computerId().isPresent());
            packet.computerId().ifPresent(uuid -> {
                buf.writeLong(uuid.getMostSignificantBits());
                buf.writeLong(uuid.getLeastSignificantBits());
            });
        }
    };

    /**
     * Convenience constructor for single character input.
     */
    public TerminalInputPacket(BlockPos pos, char c) {
        this(pos, String.valueOf(c), Optional.empty());
    }

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    /**
     * Handles the packet on the server side.
     */
    public static void handle(TerminalInputPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            try {
                if (context.player() instanceof ServerPlayer player) {
                    Level level = player.level();
                    BlockPos pos = packet.pos();

                    // Verify the player is close enough to the terminal
                    if (player.distanceToSqr(pos.getX() + 0.5, pos.getY() + 0.5, pos.getZ() + 0.5) > 64) {
                        return;
                    }

                    // Try UUID-based lookup first
                    TerminalBlockEntity te = null;
                    if (packet.computerId().isPresent()) {
                        IComputerHost host = ComputerRegistry.get(packet.computerId().get());
                        if (host instanceof TerminalBlockEntity tbe) {
                            te = tbe;
                        }
                    }
                    if (te == null) {
                        // Fall back to BlockPos lookup
                        if (level.getBlockEntity(pos) instanceof TerminalBlockEntity tbe) {
                            te = tbe;
                        }
                    }

                    if (te != null) {
                        te.onStringInput(packet.input());
                    }
                }
            } catch (Throwable e) {
                // Final safety net: prevent any unhandled exceptions from crashing the server
                // This is the last line of defense against WASM execution failures
                EvansComputerMod.LOGGER.error("Error handling terminal input packet", e);
            }
        });
    }
}
