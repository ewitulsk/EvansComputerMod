package com.example.customworld.network;

import com.example.customworld.CustomWorldMod;
import com.example.customworld.block.TerminalBlockEntity;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.BlockPos;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.level.Level;
import net.neoforged.neoforge.network.handling.IPayloadContext;

/**
 * Packet sent from client to server when the player types in the terminal.
 * Supports both single characters and escape sequences for special keys.
 */
public record TerminalInputPacket(
        BlockPos pos,
        String input
) implements CustomPacketPayload {
    
    public static final Type<TerminalInputPacket> TYPE = 
            new Type<>(ResourceLocation.fromNamespaceAndPath(CustomWorldMod.MODID, "terminal_input"));
    
    public static final StreamCodec<ByteBuf, TerminalInputPacket> STREAM_CODEC = StreamCodec.composite(
            BlockPos.STREAM_CODEC, TerminalInputPacket::pos,
            ByteBufCodecs.STRING_UTF8, TerminalInputPacket::input,
            TerminalInputPacket::new
    );
    
    /**
     * Convenience constructor for single character input.
     */
    public TerminalInputPacket(BlockPos pos, char c) {
        this(pos, String.valueOf(c));
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
                    
                    // Get the terminal block entity
                    if (level.getBlockEntity(pos) instanceof TerminalBlockEntity te) {
                        te.onStringInput(packet.input());
                    }
                }
            } catch (Throwable e) {
                // Final safety net: prevent any unhandled exceptions from crashing the server
                // This is the last line of defense against WASM execution failures
                CustomWorldMod.LOGGER.error("Error handling terminal input packet", e);
            }
        });
    }
}
