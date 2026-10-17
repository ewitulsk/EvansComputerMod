package com.example.evanscomputermod.card.applet;

import com.example.evanscomputermod.card.Hex;
import com.example.evanscomputermod.card.Redacted;
import com.mojang.serialization.Codec;
import com.mojang.serialization.DataResult;
import com.mojang.serialization.codecs.RecordCodecBuilder;
import io.netty.buffer.ByteBuf;
import net.minecraft.network.codec.StreamCodec;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/**
 * An NFC card's sectors ({@code evanscomputermod:nfc_secret}): 8 sector keys (16 bytes)
 * and 8 sectors of data (64 bytes). Saved, never sent to a client ({@link Redacted}): the
 * data is only reachable through an authenticated sector session. A blank card carries the
 * factory key in every sector and zero data.
 */
public record NfcSecret(List<String> keysHex, List<String> dataHex) {
    public static final int SECTORS = 8, KEY_BYTES = 16, DATA_BYTES = 64;
    /** Every sector of a blank card: ASCII {@code ECM-NFC-FACTORY0}. */
    public static final byte[] FACTORY_KEY = "ECM-NFC-FACTORY0".getBytes(StandardCharsets.US_ASCII);
    public static final NfcSecret HIDDEN = new NfcSecret(List.of(), List.of());

    public NfcSecret {
        keysHex = List.copyOf(keysHex == null ? List.of() : keysHex);
        dataHex = List.copyOf(dataHex == null ? List.of() : dataHex);
        if (!keysHex.isEmpty() && (keysHex.size() != SECTORS || dataHex.size() != SECTORS))
            throw new IllegalArgumentException("an NFC card has " + SECTORS + " sectors");
        for (String k : keysHex) Hex.parse(k, KEY_BYTES, "a sector key");
        for (String d : dataHex) Hex.parse(d, DATA_BYTES, "sector data");
    }

    public static NfcSecret factory() {
        List<String> k = new ArrayList<>(), d = new ArrayList<>();
        for (int i = 0; i < SECTORS; i++) {
            k.add(Hex.of(FACTORY_KEY));
            d.add(Hex.of(new byte[DATA_BYTES]));
        }
        return new NfcSecret(k, d);
    }

    public boolean known() {
        return !keysHex.isEmpty();
    }

    public byte[] key(int sector) {
        return Hex.parse(keysHex.get(sector), KEY_BYTES, "a sector key");
    }

    public byte[] data(int sector) {
        return Hex.parse(dataHex.get(sector), DATA_BYTES, "sector data");
    }

    public NfcSecret withKey(int sector, byte[] key) {
        List<String> k = new ArrayList<>(keysHex);
        k.set(sector, Hex.of(key));
        return new NfcSecret(k, dataHex);
    }

    public NfcSecret withData(int sector, byte[] data) {
        List<String> d = new ArrayList<>(dataHex);
        d.set(sector, Hex.of(data));
        return new NfcSecret(keysHex, d);
    }

    /** Whether a sector still has the factory key. */
    public boolean factoryKey(int sector) {
        return Arrays.equals(key(sector), FACTORY_KEY);
    }

    @Override
    public String toString() {
        return known() ? "NfcSecret[redacted]" : "NfcSecret[hidden]";
    }

    public static final Codec<NfcSecret> CODEC = RecordCodecBuilder.<NfcSecret>create(i -> i.group(
            Codec.STRING.listOf().fieldOf("keys").forGetter(NfcSecret::keysHex),
            Codec.STRING.listOf().fieldOf("data").forGetter(NfcSecret::dataHex)
    ).apply(i, NfcSecret::new)).flatXmap(DataResult::success, DataResult::success);

    public static final StreamCodec<ByteBuf, NfcSecret> REDACTED_STREAM_CODEC = Redacted.streamCodec(HIDDEN);

    public static net.minecraft.core.component.DataComponentType<NfcSecret> componentType() {
        return net.minecraft.core.component.DataComponentType.<NfcSecret>builder().persistent(CODEC).networkSynchronized(REDACTED_STREAM_CODEC).build();
    }
}
