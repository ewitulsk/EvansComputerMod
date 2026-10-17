package com.example.evanscomputermod.card.applet;

import com.mojang.serialization.Codec;
import io.netty.buffer.ByteBuf;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;

/**
 * The public half of an NFC card ({@code evanscomputermod:nfc_applet}): which sectors have
 * been written or rekeyed (a bitmask), for tooltips and {@code identity()}. Nothing else
 * about the sectors is public.
 */
public record NfcPublic(int writtenMask) {
    public static final NfcPublic FRESH = new NfcPublic(0);

    public boolean written(int sector) {
        return (writtenMask >> sector & 1) != 0;
    }

    public NfcPublic mark(int sector) {
        return new NfcPublic(writtenMask | 1 << sector);
    }

    public int writtenCount() {
        return Integer.bitCount(writtenMask & 0xFF);
    }

    public static final Codec<NfcPublic> CODEC = Codec.INT.fieldOf("written").codec().xmap(NfcPublic::new, NfcPublic::writtenMask);
    public static final StreamCodec<ByteBuf, NfcPublic> STREAM_CODEC = ByteBufCodecs.VAR_INT.map(NfcPublic::new, NfcPublic::writtenMask);

    public static net.minecraft.core.component.DataComponentType<NfcPublic> componentType() {
        return net.minecraft.core.component.DataComponentType.<NfcPublic>builder().persistent(CODEC).networkSynchronized(STREAM_CODEC).build();
    }
}
