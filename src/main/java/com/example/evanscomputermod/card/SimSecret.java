package com.example.evanscomputermod.card;

import com.mojang.serialization.Codec;
import com.mojang.serialization.DataResult;
import io.netty.buffer.ByteBuf;
import net.minecraft.network.codec.StreamCodec;

import java.util.HexFormat;

/**
 * The secret half of a written SIM card: its 128-bit key K. Stored on the
 * SIM item as the {@code evanscomputermod:sim_secret} component.
 *
 * <p>The key must never reach a client. Vanilla's component builder falls
 * back to the persistent codec for the network when no network codec is given
 * (which would send the key), so this component's network codec is
 * {@link #REDACTED_STREAM_CODEC}: it writes nothing and every client decodes
 * {@link #HIDDEN}. Saving (disk, the persistent codec) keeps the real key.
 * A client therefore can't tell one card's key from another's, and anything a
 * client sends back (the creative inventory) carries {@code HIDDEN}; see
 * {@code SimSecretGuard}, which puts the server's key back.
 */
public record SimSecret(String keyHex) {
    /** What clients see in place of the key. */
    public static final SimSecret HIDDEN = new SimSecret("");

    public SimSecret {
        if (keyHex == null) keyHex = "";
        if (!keyHex.isEmpty()) keyHex = HexFormat.of().formatHex(SimAuth.parseKey(keyHex));
    }

    public static SimSecret of(byte[] key) {
        if (key == null || key.length != SimAuth.KEY_BYTES) throw new IllegalArgumentException("a SIM key is 16 bytes");
        return new SimSecret(HexFormat.of().formatHex(key));
    }

    /** False for the {@link #HIDDEN} placeholder a client holds. */
    public boolean known() {
        return !keyHex.isEmpty();
    }

    /** The key bytes; only on the server (throws for {@link #HIDDEN}). */
    public byte[] key() {
        if (!known()) throw new IllegalStateException("the SIM key is not known here");
        return HexFormat.of().parseHex(keyHex);
    }

    @Override
    public String toString() {
        return known() ? "SimSecret[redacted]" : "SimSecret[hidden]";
    }

    public static final Codec<SimSecret> CODEC = Codec.STRING.fieldOf("key").codec().flatXmap(k -> {
        try {
            return DataResult.success(new SimSecret(k));
        } catch (IllegalArgumentException e) {
            return DataResult.error(() -> "bad SIM key: " + e.getMessage());
        }
    }, s -> DataResult.success(s.keyHex()));

    /** The network form: nothing at all. Clients decode {@link #HIDDEN}. */
    public static final StreamCodec<ByteBuf, SimSecret> REDACTED_STREAM_CODEC = StreamCodec.of((buf, s) -> {}, buf -> HIDDEN);

    /** The data component type (registered by CellularContent; built here so tests use the same one). */
    public static net.minecraft.core.component.DataComponentType<SimSecret> componentType() {
        return net.minecraft.core.component.DataComponentType.<SimSecret>builder().persistent(CODEC).networkSynchronized(REDACTED_STREAM_CODEC).build();
    }
}
