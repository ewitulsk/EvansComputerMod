package com.example.customworld.network;

import com.example.customworld.CustomWorldMod;
import com.example.customworld.block.TerminalBlockEntity;
import io.netty.buffer.ByteBuf;
import net.minecraft.client.Minecraft;
import net.minecraft.core.BlockPos;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.level.Level;
import net.neoforged.neoforge.network.handling.IPayloadContext;

/**
 * Packet sent from server to client to update the terminal display.
 * Contains the full terminal buffer content for synchronization.
 */
public record TerminalOutputPacket(
        BlockPos pos,
        String bufferContent,
        int cursorX,
        int cursorY
) implements CustomPacketPayload {
    
    public static final Type<TerminalOutputPacket> TYPE = 
            new Type<>(ResourceLocation.fromNamespaceAndPath(CustomWorldMod.MODID, "terminal_output"));
    
    public static final StreamCodec<ByteBuf, TerminalOutputPacket> STREAM_CODEC = StreamCodec.composite(
            BlockPos.STREAM_CODEC, TerminalOutputPacket::pos,
            ByteBufCodecs.STRING_UTF8, TerminalOutputPacket::bufferContent,
            ByteBufCodecs.INT, TerminalOutputPacket::cursorX,
            ByteBufCodecs.INT, TerminalOutputPacket::cursorY,
            TerminalOutputPacket::new
    );
    
    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }
    
    /**
     * Creates a packet from a terminal block entity.
     */
    public static TerminalOutputPacket fromBlockEntity(TerminalBlockEntity te) {
        return new TerminalOutputPacket(
                te.getBlockPos(),
                te.getBufferAsString(),
                te.getCursorX(),
                te.getCursorY()
        );
    }
    
    /**
     * Handles the packet on the client side.
     */
    public static void handle(TerminalOutputPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            Minecraft mc = Minecraft.getInstance();
            Level level = mc.level;
            
            if (level != null && level.getBlockEntity(packet.pos()) instanceof TerminalBlockEntity te) {
                te.setBufferFromString(packet.bufferContent());
                te.setCursor(packet.cursorX(), packet.cursorY());
            }
        });
    }
}
