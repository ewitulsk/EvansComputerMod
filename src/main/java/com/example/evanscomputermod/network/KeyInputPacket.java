package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.computer.ComputerInstance;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.UUIDUtil;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerPlayer;
import net.neoforged.neoforge.network.handling.IPayloadContext;

import java.nio.charset.StandardCharsets;
import java.util.UUID;

/**
 * Client to server: one raw key event (a USB HID usage with the protocol's
 * modifier bits) or typed text, for the GUI (docs/gui/UI_PROTOCOL.md §7).
 * The client sends these next to the console's key strings
 * ({@link TerminalInputPacket}); the server uses whichever matches what the
 * display shows, so console mode is exactly as before. The console/GUI
 * switch chord (Ctrl+Alt+Backspace) works from either.
 */
public record KeyInputPacket(UUID computerId, byte displayId, byte action, short hid, byte mods, String text)
        implements CustomPacketPayload {

    public static final byte DOWN = 0, UP = 1, REPEAT = 2, TEXT = 3;
    private static final int MAX_TEXT = 256;

    public static final Type<KeyInputPacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "key_input"));

    public static final StreamCodec<ByteBuf, KeyInputPacket> STREAM_CODEC = new StreamCodec<>() {
        @Override
        public KeyInputPacket decode(ByteBuf buf) {
            UUID id = UUIDUtil.STREAM_CODEC.decode(buf);
            byte display = buf.readByte();
            byte action = buf.readByte();
            short hid = buf.readShort();
            byte mods = buf.readByte();
            int n = Math.min(buf.readUnsignedShort(), MAX_TEXT * 4);
            byte[] t = new byte[n];
            buf.readBytes(t);
            return new KeyInputPacket(id, display, action, hid, mods, new String(t, StandardCharsets.UTF_8));
        }

        @Override
        public void encode(ByteBuf buf, KeyInputPacket p) {
            UUIDUtil.STREAM_CODEC.encode(buf, p.computerId());
            buf.writeByte(p.displayId());
            buf.writeByte(p.action());
            buf.writeShort(p.hid());
            buf.writeByte(p.mods());
            byte[] t = p.text().getBytes(StandardCharsets.UTF_8);
            int n = Math.min(t.length, MAX_TEXT * 4);
            buf.writeShort(n);
            buf.writeBytes(t, 0, n);
        }
    };

    public static KeyInputPacket key(UUID id, byte display, byte action, int hid, int mods) {
        return new KeyInputPacket(id, display, action, (short) hid, (byte) mods, "");
    }

    public static KeyInputPacket text(UUID id, byte display, String text) {
        return new KeyInputPacket(id, display, TEXT, (short) 0, (byte) 0, text);
    }

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    public static void handle(KeyInputPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            try {
                if (!(context.player() instanceof ServerPlayer player)) return;
                IComputerHost host = ComputerTargets.resolve(player, packet.computerId(), packet.displayId());
                if (host == null) return;
                ComputerInstance computer = ComputerTargets.instance(host);
                if (computer == null) return;
                if (packet.action() == TEXT) {
                    computer.handleTextEvent(packet.text());
                } else if (packet.action() >= DOWN && packet.action() <= REPEAT) {
                    computer.handleKeyEvent(packet.hid() & 0xFFFF, packet.mods() & 0xF,
                            packet.action() != UP, packet.action() == REPEAT);
                }
            } catch (Throwable e) {
                EvansComputerMod.LOGGER.error("Error handling key input packet", e);
            }
        });
    }
}
