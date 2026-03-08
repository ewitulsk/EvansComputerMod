package com.example.customworld.network;

import com.example.customworld.CustomWorldMod;
import com.example.customworld.block.TerminalBlockEntity;
import net.minecraft.core.BlockPos;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.level.Level;
import net.minecraftforge.network.NetworkEvent;

import java.util.function.Supplier;

/**
 * Packet sent from client to server when the player types in the terminal.
 * Supports both single characters and escape sequences for special keys.
 */
public class TerminalInputPacket {

    private final BlockPos pos;
    private final String input;

    public TerminalInputPacket(BlockPos pos, String input) {
        this.pos = pos;
        this.input = input;
    }

    /**
     * Convenience constructor for single character input.
     */
    public TerminalInputPacket(BlockPos pos, char c) {
        this(pos, String.valueOf(c));
    }

    public void encode(FriendlyByteBuf buf) {
        buf.writeBlockPos(pos);
        buf.writeUtf(input);
    }

    public static TerminalInputPacket decode(FriendlyByteBuf buf) {
        BlockPos pos = buf.readBlockPos();
        String input = buf.readUtf();
        return new TerminalInputPacket(pos, input);
    }

    public void handle(Supplier<NetworkEvent.Context> ctxSupplier) {
        NetworkEvent.Context ctx = ctxSupplier.get();
        ctx.enqueueWork(() -> {
            try {
                ServerPlayer player = ctx.getSender();
                if (player != null) {
                    Level level = player.level();

                    // Verify the player is close enough to the terminal
                    if (player.distanceToSqr(pos.getX() + 0.5, pos.getY() + 0.5, pos.getZ() + 0.5) > 64) {
                        return;
                    }

                    // Get the terminal block entity
                    if (level.getBlockEntity(pos) instanceof TerminalBlockEntity te) {
                        te.onStringInput(input);
                    }
                }
            } catch (Throwable e) {
                CustomWorldMod.LOGGER.error("Error handling terminal input packet", e);
            }
        });
        ctx.setPacketHandled(true);
    }
}
