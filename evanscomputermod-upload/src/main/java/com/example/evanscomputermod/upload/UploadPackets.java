package com.example.evanscomputermod.upload;

import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;

/**
 * The upload conversation between the server and the player's game, keyed by
 * the upload's ticket:
 * <pre>
 *   server -> game   Pick     (the computer asks for a file)
 *   game -> server   Begin    (the player picked name, size) | Cancel
 *   game -> server   Chunk*   (the bytes, in order)
 *   game -> server   End
 *   server -> game   Result   (done with the path on the computer, or why not)
 * </pre>
 * A chunk stays under the 32 KiB limit of a client-to-server payload.
 */
public final class UploadPackets {
    /** Bytes of the file in one {@link Chunk}. */
    public static final int CHUNK = 30_000;

    private UploadPackets() {}

    /** Server to game: let the player pick a file for upload {@code ticket}. */
    public record Pick(int ticket, String computerKind, String destination, long maxBytes) implements CustomPacketPayload {
        public static final Type<Pick> TYPE = Compat.type("pick");
        public static final StreamCodec<FriendlyByteBuf, Pick> STREAM_CODEC = CustomPacketPayload.codec(
                (p, b) -> {
                    b.writeVarInt(p.ticket);
                    b.writeUtf(p.computerKind, 32);
                    b.writeUtf(p.destination, 256);
                    b.writeVarLong(p.maxBytes);
                },
                b -> new Pick(b.readVarInt(), b.readUtf(32), b.readUtf(256), b.readVarLong()));

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    /** Game to server: the player picked {@code name}, {@code size} bytes long. */
    public record Begin(int ticket, String name, long size) implements CustomPacketPayload {
        public static final Type<Begin> TYPE = Compat.type("begin");
        public static final StreamCodec<FriendlyByteBuf, Begin> STREAM_CODEC = CustomPacketPayload.codec(
                (p, b) -> {
                    b.writeVarInt(p.ticket);
                    b.writeUtf(p.name, 512);
                    b.writeVarLong(p.size);
                },
                b -> new Begin(b.readVarInt(), b.readUtf(512), b.readVarLong()));

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    /** Game to server: the next bytes of the file. */
    public record Chunk(int ticket, byte[] data) implements CustomPacketPayload {
        public static final Type<Chunk> TYPE = Compat.type("chunk");
        public static final StreamCodec<FriendlyByteBuf, Chunk> STREAM_CODEC = CustomPacketPayload.codec(
                (p, b) -> {
                    b.writeVarInt(p.ticket);
                    b.writeByteArray(p.data);
                },
                b -> new Chunk(b.readVarInt(), b.readByteArray(CHUNK)));

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    /** Game to server: every byte was sent. */
    public record End(int ticket) implements CustomPacketPayload {
        public static final Type<End> TYPE = Compat.type("end");
        public static final StreamCodec<FriendlyByteBuf, End> STREAM_CODEC = CustomPacketPayload.codec(
                (p, b) -> b.writeVarInt(p.ticket), b -> new End(b.readVarInt()));

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    /** Game to server: no file after all ({@code status}: an UploadStatus error, e.g. cancelled). */
    public record Cancel(int ticket, int status) implements CustomPacketPayload {
        public static final Type<Cancel> TYPE = Compat.type("cancel");
        public static final StreamCodec<FriendlyByteBuf, Cancel> STREAM_CODEC = CustomPacketPayload.codec(
                (p, b) -> {
                    b.writeVarInt(p.ticket);
                    b.writeVarInt(p.status);
                },
                b -> new Cancel(b.readVarInt(), b.readVarInt()));

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    /** Server to game: how upload {@code ticket} ended (UploadStatus DONE with {@code path}, or an error). */
    public record Result(int ticket, int status, String path) implements CustomPacketPayload {
        public static final Type<Result> TYPE = Compat.type("result");
        public static final StreamCodec<FriendlyByteBuf, Result> STREAM_CODEC = CustomPacketPayload.codec(
                (p, b) -> {
                    b.writeVarInt(p.ticket);
                    b.writeVarInt(p.status);
                    b.writeUtf(p.path, 512);
                },
                b -> new Result(b.readVarInt(), b.readVarInt(), b.readUtf(512)));

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }
}
