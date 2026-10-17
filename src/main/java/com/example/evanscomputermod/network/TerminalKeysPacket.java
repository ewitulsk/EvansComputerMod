package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.UUIDUtil;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;
import net.neoforged.neoforge.network.handling.IPayloadContext;

import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Server to client: whether a full-screen program (vim, the pager, ...) holds a
 * computer's keyboard (the kernel's framebuffer flag, {@code program_keys}).
 * While it does, the Terminal screen sends plain Esc to the program and
 * Shift+Esc closes the screen, whatever the screen does with Esc otherwise.
 * Sent when the flag changes and when a player opens the Terminal.
 */
public record TerminalKeysPacket(UUID computerId, boolean programKeys) implements CustomPacketPayload {
    public static final Type<TerminalKeysPacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "terminal_keys"));
    public static final StreamCodec<ByteBuf, TerminalKeysPacket> STREAM_CODEC = StreamCodec.composite(
            UUIDUtil.STREAM_CODEC, TerminalKeysPacket::computerId,
            ByteBufCodecs.BOOL, TerminalKeysPacket::programKeys,
            TerminalKeysPacket::new);

    /** Client side: the computers whose keyboard a full-screen program holds. */
    private static final Map<UUID, Boolean> HELD = new ConcurrentHashMap<>();

    /** Client side: does a full-screen program hold {@code computerId}'s keyboard? */
    public static boolean programHoldsKeys(UUID computerId) {
        return computerId != null && HELD.getOrDefault(computerId, false);
    }

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    public static void handle(TerminalKeysPacket p, IPayloadContext ctx) {
        ctx.enqueueWork(() -> {
            if (p.programKeys()) HELD.put(p.computerId(), true);
            else HELD.remove(p.computerId());
        });
    }
}
