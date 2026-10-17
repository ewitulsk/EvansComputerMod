package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.computer.ComputerInstance;
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
 * Packet sent from client to server when the player types into a computer's
 * display. Supports both single characters and escape sequences for special
 * keys. Addressed by computer id and the display the player typed into.
 */
public record TerminalInputPacket(
        UUID computerId,
        byte displayId,
        String input
) implements CustomPacketPayload {

    public static final Type<TerminalInputPacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "terminal_input"));

    public static final StreamCodec<ByteBuf, TerminalInputPacket> STREAM_CODEC = StreamCodec.composite(
            UUIDUtil.STREAM_CODEC, TerminalInputPacket::computerId,
            ByteBufCodecs.BYTE, TerminalInputPacket::displayId,
            ByteBufCodecs.STRING_UTF8, TerminalInputPacket::input,
            TerminalInputPacket::new
    );

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    /**
     * Hand typed keys from {@code player} to the computer the packet names, if
     * the host takes input from that player on that display (range and holder
     * rules, {@code IComputerHost.acceptsInput}). Server thread. Scenario
     * players type through this too. Returns false if refused.
     */
    public static boolean deliver(ServerPlayer player, TerminalInputPacket packet) {
        IComputerHost host = ComputerTargets.resolveInput(player, packet.computerId(), packet.displayId());
        if (host == null) return false;
        ComputerInstance computer = ComputerTargets.instance(host);
        if (computer == null) return false;
        computer.handleKeyInput(packet.input());
        host.markDirty();
        return true;
    }

    /**
     * Handles the packet on the server side.
     */
    public static void handle(TerminalInputPacket packet, IPayloadContext context) {
        // Network thread (DISPLAY_PIPELINE_SPEC.md Part A3): straight to the kernel's input queue.
        try {
            if (!(context.player() instanceof ServerPlayer player)) return;
            ComputerTargets.input(player, packet.computerId(), packet.displayId(),
                    computer -> computer.handleKeyInput(packet.input()), () -> {
                        try {
                            deliver(player, packet);
                        } catch (Throwable e) {
                            EvansComputerMod.LOGGER.error("Error handling terminal input packet", e);
                        }
                    });
        } catch (Throwable e) {
            // Final safety net: prevent any unhandled exceptions from crashing the server
            // This is the last line of defense against WASM execution failures
            EvansComputerMod.LOGGER.error("Error handling terminal input packet", e);
        }
    }
}
