package com.example.evanscomputermod.storage.item;

//? if <=1.21.1 {

import com.mojang.serialization.Codec;
import com.mojang.serialization.codecs.RecordCodecBuilder;
import io.netty.buffer.ByteBuf;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;

/**
 * Last known fill of a cell, copied onto its item so tooltips work on the
 * client (the ledger itself is server-only). Informational: the ledger is the
 * authority.
 */
public record CellSummary(int types, long items, long bytes) {

    public static final Codec<CellSummary> CODEC = RecordCodecBuilder.create(i -> i.group(
            Codec.INT.fieldOf("types").forGetter(CellSummary::types),
            Codec.LONG.fieldOf("items").forGetter(CellSummary::items),
            Codec.LONG.fieldOf("bytes").forGetter(CellSummary::bytes)
    ).apply(i, CellSummary::new));

    public static final StreamCodec<ByteBuf, CellSummary> STREAM_CODEC = StreamCodec.composite(
            ByteBufCodecs.VAR_INT, CellSummary::types,
            ByteBufCodecs.VAR_LONG, CellSummary::items,
            ByteBufCodecs.VAR_LONG, CellSummary::bytes,
            CellSummary::new);
}
//?}
