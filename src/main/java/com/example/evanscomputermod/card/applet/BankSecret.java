package com.example.evanscomputermod.card.applet;

import com.example.evanscomputermod.card.Hex;
import com.example.evanscomputermod.card.Redacted;
import com.mojang.serialization.Codec;
import com.mojang.serialization.DataResult;
import com.mojang.serialization.codecs.RecordCodecBuilder;
import io.netty.buffer.ByteBuf;
import net.minecraft.network.codec.StreamCodec;

/**
 * The secret half of a bank chip ({@code evanscomputermod:bank_secret}): the 32-byte
 * signing key K and the PIN hash. Saved, never sent to a client ({@link Redacted}).
 */
public record BankSecret(String keyHex, String pinHashHex) {
    public static final int KEY_BYTES = 32;
    /** What clients hold in place of the secret. */
    public static final BankSecret HIDDEN = new BankSecret("", "");

    public BankSecret {
        if (keyHex == null) keyHex = "";
        if (pinHashHex == null) pinHashHex = "";
        if (!keyHex.isEmpty()) keyHex = Hex.of(Hex.parse(keyHex, KEY_BYTES, "a bank key"));
        if (!pinHashHex.isEmpty()) pinHashHex = Hex.of(Hex.parse(pinHashHex, 32, "a PIN hash"));
    }

    public static BankSecret of(byte[] key, byte[] pinHash) {
        return new BankSecret(Hex.of(key), pinHash == null ? "" : Hex.of(pinHash));
    }

    public boolean known() {
        return !keyHex.isEmpty();
    }

    public byte[] key() {
        if (!known()) throw new IllegalStateException("the bank key is not known here");
        return Hex.parse(keyHex, KEY_BYTES, "a bank key");
    }

    public byte[] pinHash() {
        return pinHashHex.isEmpty() ? null : Hex.parse(pinHashHex, 32, "a PIN hash");
    }

    @Override
    public String toString() {
        return known() ? "BankSecret[redacted]" : "BankSecret[hidden]";
    }

    public static final Codec<BankSecret> CODEC = RecordCodecBuilder.<BankSecret>create(i -> i.group(
            Codec.STRING.fieldOf("key").forGetter(BankSecret::keyHex),
            Codec.STRING.optionalFieldOf("pin", "").forGetter(BankSecret::pinHashHex)
    ).apply(i, BankSecret::new)).flatXmap(DataResult::success, s -> DataResult.success(s));

    public static final StreamCodec<ByteBuf, BankSecret> REDACTED_STREAM_CODEC = Redacted.streamCodec(HIDDEN);

    public static net.minecraft.core.component.DataComponentType<BankSecret> componentType() {
        return net.minecraft.core.component.DataComponentType.<BankSecret>builder().persistent(CODEC).networkSynchronized(REDACTED_STREAM_CODEC).build();
    }
}
