package com.example.customworld.network;

import com.example.customworld.block.TerminalBlockEntity;
import net.minecraft.client.Minecraft;
import net.minecraft.core.BlockPos;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.world.level.Level;
import net.minecraftforge.network.NetworkEvent;

import java.util.function.Supplier;

/**
 * Packet sent from server to client to update the terminal display.
 * Contains the full terminal buffer content for synchronization.
 */
public class TerminalOutputPacket {

    private final BlockPos pos;
    private final String bufferContent;
    private final int cursorX;
    private final int cursorY;

    public TerminalOutputPacket(BlockPos pos, String bufferContent, int cursorX, int cursorY) {
        this.pos = pos;
        this.bufferContent = bufferContent;
        this.cursorX = cursorX;
        this.cursorY = cursorY;
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

    public void encode(FriendlyByteBuf buf) {
        buf.writeBlockPos(pos);
        buf.writeUtf(bufferContent);
        buf.writeInt(cursorX);
        buf.writeInt(cursorY);
    }

    public static TerminalOutputPacket decode(FriendlyByteBuf buf) {
        BlockPos pos = buf.readBlockPos();
        String bufferContent = buf.readUtf();
        int cursorX = buf.readInt();
        int cursorY = buf.readInt();
        return new TerminalOutputPacket(pos, bufferContent, cursorX, cursorY);
    }

    public void handle(Supplier<NetworkEvent.Context> ctxSupplier) {
        NetworkEvent.Context ctx = ctxSupplier.get();
        ctx.enqueueWork(() -> {
            Minecraft mc = Minecraft.getInstance();
            Level level = mc.level;

            if (level != null && level.getBlockEntity(pos) instanceof TerminalBlockEntity te) {
                te.setBufferFromString(bufferContent);
                te.setCursor(cursorX, cursorY);
            }
        });
        ctx.setPacketHandled(true);
    }
}
