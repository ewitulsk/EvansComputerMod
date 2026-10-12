package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.computer.ui.UiClients;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.UUIDUtil;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;
import net.minecraft.server.level.ServerPlayer;
import net.neoforged.neoforge.network.handling.IPayloadContext;

import java.util.UUID;

/**
 * Packets of client-side GUI rendering (docs/gui/UI_PROTOCOL.md §14 phase
 * 2a, §18). The scene stream's messages themselves are
 * {@link com.example.evanscomputermod.computer.ui.UiSceneCodec} bytes; these
 * records only address and carry them.
 */
public final class UiScenePackets {
    private UiScenePackets() {}

    /** Largest scene message accepted (a keyframe of a full scene, or one image). */
    static final int MAX_DATA = 32 << 20;
    /** Most image keys in one ack or hello. */
    static final int MAX_KEYS = 4096;

    private static void writeKeys(ByteBuf buf, long[] keys) {
        int n = keys == null ? 0 : Math.min(keys.length, MAX_KEYS);
        buf.writeShort(n);
        for (int i = 0; i < n; i++) buf.writeLong(keys[i]);
    }

    static void writeVar(ByteBuf buf, int v) {
        while ((v & ~0x7F) != 0) {
            buf.writeByte((v & 0x7F) | 0x80);
            v >>>= 7;
        }
        buf.writeByte(v);
    }

    static int readVar(ByteBuf buf) {
        int v = 0;
        for (int shift = 0; shift < 35; shift += 7) {
            int b = buf.readByte();
            v |= (b & 0x7F) << shift;
            if ((b & 0x80) == 0) return v;
        }
        throw new IllegalArgumentException("varint too long");
    }

    private static long[] readKeys(ByteBuf buf) {
        int n = Math.min(buf.readUnsignedShort(), MAX_KEYS);
        long[] k = new long[n];
        for (int i = 0; i < n; i++) k[i] = buf.readLong();
        return k;
    }

    /**
     * Server to client: one scene-stream message for a computer display, fully
     * addressed; it also binds the short {@code stream} id later {@link Step}s use.
     */
    public record Scene(UUID computerId, byte displayId, int stream, byte[] data) implements CustomPacketPayload {
        public static final Type<Scene> TYPE =
                new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "ui_scene"));

        public static final StreamCodec<ByteBuf, Scene> STREAM_CODEC = new StreamCodec<>() {
            @Override
            public Scene decode(ByteBuf buf) {
                UUID id = UUIDUtil.STREAM_CODEC.decode(buf);
                byte display = buf.readByte();
                int stream = readVar(buf);
                int n = buf.readInt();
                if (n < 0 || n > MAX_DATA) throw new IllegalArgumentException("scene message of " + n + " bytes");
                byte[] d = new byte[n];
                buf.readBytes(d);
                return new Scene(id, display, stream, d);
            }

            @Override
            public void encode(ByteBuf buf, Scene p) {
                UUIDUtil.STREAM_CODEC.encode(buf, p.computerId());
                buf.writeByte(p.displayId());
                writeVar(buf, p.stream());
                buf.writeInt(p.data().length);
                buf.writeBytes(p.data());
            }
        };

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    /**
     * Server to client: one scene step (the common, small message) addressed by
     * the short stream id a {@link Scene} bound, so typing costs tens of bytes.
     */
    public record Step(int stream, byte[] data) implements CustomPacketPayload {
        public static final Type<Step> TYPE =
                new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "ui_step"));

        public static final StreamCodec<ByteBuf, Step> STREAM_CODEC = new StreamCodec<>() {
            @Override
            public Step decode(ByteBuf buf) {
                int stream = readVar(buf);
                int n = readVar(buf);
                if (n < 0 || n > MAX_DATA) throw new IllegalArgumentException("scene step of " + n + " bytes");
                byte[] d = new byte[n];
                buf.readBytes(d);
                return new Step(stream, d);
            }

            @Override
            public void encode(ByteBuf buf, Step p) {
                writeVar(buf, p.stream());
                writeVar(buf, p.data().length);
                buf.writeBytes(p.data());
            }
        };

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    /** The packet for a scene message to {@code player}: a short {@link Step} for steps, else a full {@link Scene}. */
    public static CustomPacketPayload packet(UUID player, UUID computerId, int displayId, byte[] data) {
        int stream = UiClients.stream(player, computerId, displayId);
        if (com.example.evanscomputermod.computer.ui.UiSceneCodec.kindOf(data)
                == com.example.evanscomputermod.computer.ui.UiSceneCodec.STEP && stream != 0) {
            return new Step(stream, data);
        }
        return new Scene(computerId, (byte) displayId, stream, data);
    }

    /**
     * Client to server: the client applied scene generation {@code generation}
     * of {@code epoch}; {@code generation <= 0} asks for a keyframe and
     * {@code missing} lists image keys it doesn't hold.
     */
    public record Ack(UUID computerId, byte displayId, int epoch, long generation, long[] missing)
            implements CustomPacketPayload {
        public static final Type<Ack> TYPE =
                new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "ui_scene_ack"));

        public static final StreamCodec<ByteBuf, Ack> STREAM_CODEC = new StreamCodec<>() {
            @Override
            public Ack decode(ByteBuf buf) {
                return new Ack(UUIDUtil.STREAM_CODEC.decode(buf), buf.readByte(), buf.readInt(), buf.readLong(), readKeys(buf));
            }

            @Override
            public void encode(ByteBuf buf, Ack p) {
                UUIDUtil.STREAM_CODEC.encode(buf, p.computerId());
                buf.writeByte(p.displayId());
                buf.writeInt(p.epoch());
                buf.writeLong(p.generation());
                writeKeys(buf, p.missing());
            }
        };

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }

        public static void handle(Ack packet, IPayloadContext context) {
            context.enqueueWork(() -> {
                if (context.player() instanceof ServerPlayer player) {
                    IComputerHost host = ComputerTargets.resolve(player, packet.computerId(), packet.displayId());
                    if (host != null) {
                        host.onSceneAck(player, packet.displayId(), packet.epoch(), packet.generation(), packet.missing());
                    }
                }
            });
        }
    }

    /**
     * Client to server, on joining: this client can draw scenes with stream
     * version {@code version} and already holds these image keys. A client
     * that never sends it gets tile deltas.
     */
    public record Hello(int version, long[] held) implements CustomPacketPayload {
        public static final Type<Hello> TYPE =
                new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "ui_client_hello"));

        public static final StreamCodec<ByteBuf, Hello> STREAM_CODEC = new StreamCodec<>() {
            @Override
            public Hello decode(ByteBuf buf) {
                return new Hello(buf.readInt(), readKeys(buf));
            }

            @Override
            public void encode(ByteBuf buf, Hello p) {
                buf.writeInt(p.version());
                writeKeys(buf, p.held());
            }
        };

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }

        public static void handle(Hello packet, IPayloadContext context) {
            context.enqueueWork(() -> {
                if (context.player() instanceof ServerPlayer player) {
                    UiClients.hello(player.getUUID(), packet.version(), packet.held());
                    EvansComputerMod.LOGGER.debug("{}: client GUI rendering, scene stream v{}, {} cached images",
                            player.getName().getString(), packet.version(), packet.held().length);
                }
            });
        }
    }
}
