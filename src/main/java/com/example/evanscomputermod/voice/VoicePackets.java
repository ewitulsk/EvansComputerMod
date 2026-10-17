package com.example.evanscomputermod.voice;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.speaker.ImaAdpcm;
import io.netty.buffer.ByteBuf;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.ResourceLocation;

/** Network payloads of the microphone: the player's voice up, and whether anything is listening down. */
public final class VoicePackets {
    private VoicePackets() {}

    /** Rate the client sends its microphone at. */
    public static final int VOICE_RATE = 24_000;
    /** Largest block accepted (samples): a little over a quarter second. */
    public static final int MAX_SAMPLES = 8192;

    /**
     * Client to server: one block of the player's microphone, {@code samples}
     * mono samples at {@code rate}, IMA ADPCM. {@code talk} is true while the
     * player holds the Talk key (a radio's push-to-talk).
     */
    public record Voice(int seq, int rate, int samples, byte[] data, boolean talk) implements CustomPacketPayload {
        public static final Type<Voice> TYPE = new Type<>(ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "voice"));
        public static final StreamCodec<ByteBuf, Voice> STREAM_CODEC = new StreamCodec<>() {
            @Override
            public Voice decode(ByteBuf buf) {
                int seq = buf.readInt(), rate = buf.readInt(), samples = buf.readUnsignedShort(), len = buf.readUnsignedShort();
                if (samples > MAX_SAMPLES || len > ImaAdpcm.encodedSize(MAX_SAMPLES)) throw new IllegalArgumentException("voice block too large");
                byte[] data = new byte[len];
                buf.readBytes(data);
                return new Voice(seq, rate, samples, data, buf.readBoolean());
            }

            @Override
            public void encode(ByteBuf buf, Voice v) {
                buf.writeInt(v.seq).writeInt(v.rate).writeShort(v.samples).writeShort(v.data.length);
                buf.writeBytes(v.data);
                buf.writeBoolean(v.talk);
            }
        };

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    /**
     * Client to server: the Talk key went down or up with nothing to send (the
     * microphone is off or another voice mod is quiet), so a radio still keys up
     * or unkeys.
     */
    public record Talk(boolean down) implements CustomPacketPayload {
        public static final Type<Talk> TYPE = new Type<>(ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "voice_talk"));
        public static final StreamCodec<ByteBuf, Talk> STREAM_CODEC = StreamCodec.composite(ByteBufCodecs.BOOL, Talk::down, Talk::new);

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    /** {@link Demand} bit: a microphone near you is recording. */
    public static final int DEMAND_MIC = 1;
    /** {@link Demand} bit: you hold a radio that can transmit (the Talk key keys it). */
    public static final int DEMAND_RADIO = 2;
    /** {@link Demand} bit: the radio is transmitting (you hold Talk). */
    public static final int DEMAND_ON_AIR = 4;

    /** Server to client: what wants your voice now ({@code DEMAND_*} bits); 0 = nothing. */
    public record Demand(int flags) implements CustomPacketPayload {
        public static final Type<Demand> TYPE = new Type<>(ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "voice_demand"));
        public static final StreamCodec<ByteBuf, Demand> STREAM_CODEC = StreamCodec.composite(ByteBufCodecs.VAR_INT, Demand::flags, Demand::new);

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }
}
//?}
