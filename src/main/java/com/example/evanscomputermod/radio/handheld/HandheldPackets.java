package com.example.evanscomputermod.radio.handheld;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import io.netty.buffer.ByteBuf;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.ResourceLocation;

/** Network payloads of the handheld receiver. */
public final class HandheldPackets {
    private HandheldPackets() {}

    /** Server to client: one block of demodulated audio (IMA ADPCM) plus the meter reading. */
    public record Audio(int seq, int rate, int samples, byte[] data, float signalDbm, double freqHz) implements CustomPacketPayload {
        public static final Type<Audio> TYPE = new Type<>(ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "handheld_audio"));
        public static final StreamCodec<ByteBuf, Audio> STREAM_CODEC = new StreamCodec<>() {
            @Override
            public Audio decode(ByteBuf buf) {
                int seq = buf.readInt(), rate = buf.readInt(), samples = buf.readUnsignedShort(), len = buf.readUnsignedShort();
                if (samples > 8192 || len > 8192) throw new IllegalArgumentException("handheld audio block too large");
                byte[] data = new byte[len];
                buf.readBytes(data);
                return new Audio(seq, rate, samples, data, buf.readFloat(), buf.readDouble());
            }

            @Override
            public void encode(ByteBuf buf, Audio a) {
                buf.writeInt(a.seq).writeInt(a.rate).writeShort(a.samples).writeShort(a.data.length);
                buf.writeBytes(a.data);
                buf.writeFloat(a.signalDbm).writeDouble(a.freqHz);
            }
        };

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    /** Client to server: seek the next (direction 1) or previous (-1) station on the band. */
    public record Seek(boolean mainHand, int direction) implements CustomPacketPayload {
        public static final Type<Seek> TYPE = new Type<>(ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "handheld_seek"));
        public static final StreamCodec<ByteBuf, Seek> STREAM_CODEC = StreamCodec.composite(
                ByteBufCodecs.BOOL, Seek::mainHand, ByteBufCodecs.VAR_INT, Seek::direction, Seek::new);

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    /** Server to client: where a seek landed ({@code found} false: nothing on the band, frequency unchanged). */
    public record SeekResult(boolean found, double freqHz) implements CustomPacketPayload {
        public static final Type<SeekResult> TYPE = new Type<>(ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "handheld_seek_result"));
        public static final StreamCodec<ByteBuf, SeekResult> STREAM_CODEC = StreamCodec.composite(
                ByteBufCodecs.BOOL, SeekResult::found, ByteBufCodecs.DOUBLE, SeekResult::freqHz, SeekResult::new);

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    /** Client to server: new settings for the handheld in a hand (from the tuning screen). */
    public record Settings(boolean mainHand, boolean on, int band, double freqHz, int volume, int squelch) implements CustomPacketPayload {
        public static final Type<Settings> TYPE = new Type<>(ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "handheld_settings"));
        public static final StreamCodec<ByteBuf, Settings> STREAM_CODEC = StreamCodec.composite(
                ByteBufCodecs.BOOL, Settings::mainHand, ByteBufCodecs.BOOL, Settings::on, ByteBufCodecs.VAR_INT, Settings::band,
                ByteBufCodecs.DOUBLE, Settings::freqHz, ByteBufCodecs.VAR_INT, Settings::volume, ByteBufCodecs.VAR_INT, Settings::squelch,
                Settings::new);

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }
}
//?}
