package com.example.evanscomputermod.speaker;

import com.example.evanscomputermod.EvansComputerMod;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.BlockPos;
import net.minecraft.core.UUIDUtil;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;

import org.jetbrains.annotations.Nullable;
import java.util.UUID;

/**
 * Server to client: one block of a speaker's audio, {@code samples} mono
 * samples at {@code rate} Hz, IMA ADPCM ({@link ImaAdpcm}). {@code seq}
 * counts blocks per speaker so the client can tell a gap from a reorder.
 *
 * <p>{@code ptsUs} is when the block's first sample plays on the server's
 * display clock (DISPLAY_PIPELINE_SPEC.md Part E2): the speaker's own queue
 * says how much is still ahead of it. {@code computer} is the computer the
 * speaker is attached to (null: none); while that computer streams timed
 * frames, the client plays the block with the same playout delay as the
 * picture, so sound and picture stay together.
 */
public record SpeakerAudioPacket(BlockPos pos, int seq, int rate, int samples, byte[] data, long ptsUs,
                                 @Nullable UUID computer)
        implements CustomPacketPayload {

    public static final Type<SpeakerAudioPacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "speaker_audio"));

    /** Largest block accepted (samples). */
    public static final int MAX_SAMPLES = 8192;

    /** An untimed block of no computer (as before Part E2). */
    public SpeakerAudioPacket(BlockPos pos, int seq, int rate, int samples, byte[] data) {
        this(pos, seq, rate, samples, data, 0, null);
    }

    public static final StreamCodec<ByteBuf, SpeakerAudioPacket> STREAM_CODEC = new StreamCodec<>() {
        @Override
        public SpeakerAudioPacket decode(ByteBuf buf) {
            BlockPos pos = BlockPos.STREAM_CODEC.decode(buf);
            int seq = buf.readInt();
            int rate = buf.readInt();
            int samples = buf.readUnsignedShort();
            int len = buf.readUnsignedShort();
            if (samples > MAX_SAMPLES || len > ImaAdpcm.encodedSize(MAX_SAMPLES)) {
                throw new IllegalArgumentException("speaker audio block too large");
            }
            byte[] data = new byte[len];
            buf.readBytes(data);
            long pts = buf.readLong();
            UUID computer = buf.readBoolean() ? UUIDUtil.STREAM_CODEC.decode(buf) : null;
            return new SpeakerAudioPacket(pos, seq, rate, samples, data, pts, computer);
        }

        @Override
        public void encode(ByteBuf buf, SpeakerAudioPacket p) {
            BlockPos.STREAM_CODEC.encode(buf, p.pos());
            buf.writeInt(p.seq());
            buf.writeInt(p.rate());
            buf.writeShort(p.samples());
            buf.writeShort(p.data().length);
            buf.writeBytes(p.data());
            buf.writeLong(p.ptsUs());
            buf.writeBoolean(p.computer() != null);
            if (p.computer() != null) UUIDUtil.STREAM_CODEC.encode(buf, p.computer());
        }
    };

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }
}
