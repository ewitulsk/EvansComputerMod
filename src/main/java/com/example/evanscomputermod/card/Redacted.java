package com.example.evanscomputermod.card;

import io.netty.buffer.ByteBuf;
import net.minecraft.network.codec.StreamCodec;

/**
 * Network codecs for a card's secret components: they write nothing and every client
 * decodes the {@code hidden} placeholder. Vanilla's component builder falls back to the
 * persistent codec for the network when no stream codec is given (which would send the
 * key), so every {@code *_secret} component is built with one of these. Saving keeps the
 * real value. See {@link SimSecret} for the original and {@link CardSecretGuard} for how
 * the server puts secrets back on creative-moved stacks.
 */
public final class Redacted {
    private Redacted() {}

    public static <T> StreamCodec<ByteBuf, T> streamCodec(T hidden) {
        return StreamCodec.of((buf, s) -> {}, buf -> hidden);
    }
}
