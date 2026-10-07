package com.example.evanscomputermod.speaker;

import com.example.evanscomputermod.EvansComputerMod;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.BlockPos;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;

/**
 * Server to client: one block of a speaker's audio, {@code samples} mono
 * samples at {@code rate} Hz, IMA ADPCM ({@link ImaAdpcm}). {@code seq}
 * counts blocks per speaker so the client can tell a gap from a reorder.
 */
public record SpeakerAudioPacket(BlockPos pos, int seq, int rate, int samples, byte[] data)
        implements CustomPacketPayload {

    public static final Type<SpeakerAudioPacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "speaker_audio"));

    /** Largest block accepted (samples). */
    public static final int MAX_SAMPLES = 8192;

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
            return new SpeakerAudioPacket(pos, seq, rate, samples, data);
        }

        @Override
        public void encode(ByteBuf buf, SpeakerAudioPacket p) {
            BlockPos.STREAM_CODEC.encode(buf, p.pos());
            buf.writeInt(p.seq());
            buf.writeInt(p.rate());
            buf.writeShort(p.samples());
            buf.writeShort(p.data().length);
            buf.writeBytes(p.data());
        }
    };

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }
}
