package com.example.evanscomputermod.bluetooth;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.speaker.ImaAdpcm;
import io.netty.buffer.ByteBuf;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.ResourceLocation;

/** Network payloads of Bluetooth audio and the AirPods' buttons. */
public final class BluetoothPackets {
    private BluetoothPackets() {}

    /** Client to server: the player tapped their AirPods ({@code gesture}: BtFrame.GESTURE_DOUBLE_TAP). */
    public record Tap(int gesture) implements CustomPacketPayload {
        public static final Type<Tap> TYPE = new Type<>(ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "airpods_tap"));
        public static final StreamCodec<ByteBuf, Tap> STREAM_CODEC = new StreamCodec<>() {
            @Override
            public Tap decode(ByteBuf buf) {
                return new Tap(buf.readUnsignedByte());
            }

            @Override
            public void encode(ByteBuf buf, Tap v) {
                buf.writeByte(v.gesture);
            }
        };

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    /** Largest block accepted (samples). */
    public static final int MAX_SAMPLES = 8192;

    /**
     * Server to client: one block of audio the wearer's AirPods play (only the
     * wearer hears it), {@code samples} mono samples at {@code rate}, IMA ADPCM.
     */
    public record EarAudio(int seq, int rate, int samples, byte[] data) implements CustomPacketPayload {
        public static final Type<EarAudio> TYPE = new Type<>(ResourceLocation.fromNamespaceAndPath(EvansComputerMod.MODID, "ear_audio"));
        public static final StreamCodec<ByteBuf, EarAudio> STREAM_CODEC = new StreamCodec<>() {
            @Override
            public EarAudio decode(ByteBuf buf) {
                int seq = buf.readInt(), rate = buf.readInt(), samples = buf.readUnsignedShort(), len = buf.readUnsignedShort();
                if (samples > MAX_SAMPLES || len > ImaAdpcm.encodedSize(MAX_SAMPLES)) throw new IllegalArgumentException("ear audio block too large");
                byte[] data = new byte[len];
                buf.readBytes(data);
                return new EarAudio(seq, rate, samples, data);
            }

            @Override
            public void encode(ByteBuf buf, EarAudio v) {
                buf.writeInt(v.seq).writeInt(v.rate).writeShort(v.samples).writeShort(v.data.length);
                buf.writeBytes(v.data);
            }
        };

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }
}
//?}
