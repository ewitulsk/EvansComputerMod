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
 * The GUI's clipboard and the player's real one. When the player presses a
 * paste key on a computer's screen, the client first sends its clipboard
 * ({@link Push}), so the app's paste reads what the player copied outside
 * the game. When an app copies, the server sends the text to the player who
 * is using the computer ({@link Set}), and the client puts it on the real
 * clipboard.
 */
public final class ClipboardPackets {
    /** Longest clipboard text carried (characters). */
    public static final int MAX_TEXT = 32 * 1024;

    private ClipboardPackets() {
    }

    private static String cut(String s) {
        return s.length() > MAX_TEXT ? s.substring(0, MAX_TEXT) : s;
    }

    /** Client to server: the player's clipboard, sent right before a paste key. */
    public record Push(UUID computerId, byte displayId, String text) implements CustomPacketPayload {
        public static final Type<Push> TYPE = new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "clipboard_push"));
        public static final StreamCodec<ByteBuf, Push> STREAM_CODEC = StreamCodec.composite(
                UUIDUtil.STREAM_CODEC, Push::computerId,
                ByteBufCodecs.BYTE, Push::displayId,
                ByteBufCodecs.stringUtf8(MAX_TEXT * 4), Push::text,
                Push::new);

        public Push {
            text = cut(text == null ? "" : text);
        }

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }

        /** Give the computer {@code player}'s clipboard (the host's input rules apply). Server thread. */
        public static boolean deliver(ServerPlayer player, Push p) {
            IComputerHost host = ComputerTargets.resolveInput(player, p.computerId(), p.displayId());
            if (host == null) return false;
            ComputerInstance computer = ComputerTargets.instance(host);
            if (computer == null) return false;
            computer.clipboardFromPlayer(player.getUUID(), p.text());
            return true;
        }

        public static void handle(Push p, IPayloadContext ctx) {
            ctx.enqueueWork(() -> {
                if (ctx.player() instanceof ServerPlayer player) deliver(player, p);
            });
        }
    }

    /** Server to client: text an app copied, for the player's real clipboard. */
    public record Set(String text) implements CustomPacketPayload {
        public static final Type<Set> TYPE = new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "clipboard_set"));
        public static final StreamCodec<ByteBuf, Set> STREAM_CODEC = StreamCodec.composite(
                ByteBufCodecs.stringUtf8(MAX_TEXT * 4), Set::text,
                Set::new);

        public Set {
            text = cut(text == null ? "" : text);
        }

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }

        public static void handle(Set p, IPayloadContext ctx) {
            ctx.enqueueWork(() -> com.example.evanscomputermod.client.ClientPacketHandler.setClipboard(p.text()));
        }
    }
}
