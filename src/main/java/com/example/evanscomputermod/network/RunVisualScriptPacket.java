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
import net.minecraft.server.level.ServerLevel;
import net.neoforged.neoforge.network.handling.IPayloadContext;

import java.util.Optional;
import java.util.UUID;

/**
 * Packet sent from client to server to run a visual program on a terminal.
 * Contains the generated Python code and the terminal's block position.
 */
public record RunVisualScriptPacket(BlockPos pos, String pythonCode, Optional<UUID> computerId) implements CustomPacketPayload {

    public static final Type<RunVisualScriptPacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "run_visual_script"));

    public static final StreamCodec<ByteBuf, RunVisualScriptPacket> STREAM_CODEC = new StreamCodec<>() {
        @Override
        public RunVisualScriptPacket decode(ByteBuf buf) {
            BlockPos pos = BlockPos.STREAM_CODEC.decode(buf);
            String pythonCode = ByteBufCodecs.STRING_UTF8.decode(buf);
            boolean hasUuid = buf.readBoolean();
            Optional<UUID> computerId = hasUuid ? Optional.of(new UUID(buf.readLong(), buf.readLong())) : Optional.empty();
            return new RunVisualScriptPacket(pos, pythonCode, computerId);
        }

        @Override
        public void encode(ByteBuf buf, RunVisualScriptPacket packet) {
            BlockPos.STREAM_CODEC.encode(buf, packet.pos());
            ByteBufCodecs.STRING_UTF8.encode(buf, packet.pythonCode());
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

    public static void handle(RunVisualScriptPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            var player = context.player();
            if (player.level() instanceof ServerLevel serverLevel) {
                // Try UUID-based lookup first
                TerminalBlockEntity terminal = null;
                if (packet.computerId().isPresent()) {
                    IComputerHost host = ComputerRegistry.get(packet.computerId().get());
                    if (host instanceof TerminalBlockEntity tbe) {
                        terminal = tbe;
                    }
                }
                if (terminal == null) {
                    // Fall back to BlockPos lookup
                    var blockEntity = serverLevel.getBlockEntity(packet.pos());
                    if (blockEntity instanceof TerminalBlockEntity tbe) {
                        terminal = tbe;
                    }
                }

                if (terminal != null) {
                    terminal.runVisualScript(packet.pythonCode());
                }
            }
        });
    }
}
