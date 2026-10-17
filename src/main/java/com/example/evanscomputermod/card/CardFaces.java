package com.example.evanscomputermod.card;

import com.mojang.serialization.Codec;
import com.mojang.serialization.DataResult;
import com.mojang.serialization.codecs.RecordCodecBuilder;
import io.netty.buffer.ByteBuf;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/**
 * The painted faces of a card ({@code evanscomputermod:card_faces}): a palette of up to 16
 * RGB colours and two palette-indexed bitmaps, front and back, packed two texels per byte
 * (high nibble first, row-major, width from the card's {@link CardKind}). Public and
 * synced: the renderer draws it, the printer writes it. The kind's fixed features (chip
 * pad, stripe) are composited over it by the renderer, whatever the data says there.
 */
public record CardFaces(int[] palette, byte[] front, byte[] back) {
    public static final int MAX_PALETTE = 16;

    public CardFaces {
        if (palette == null || palette.length == 0 || palette.length > MAX_PALETTE)
            throw new IllegalArgumentException("a palette has 1-" + MAX_PALETTE + " colours");
        if (front == null || back == null) throw new IllegalArgumentException("two faces");
        palette = palette.clone();
        for (int i = 0; i < palette.length; i++) palette[i] &= 0xFFFFFF;
    }

    /** Packed length of a face of {@code texels} texels. */
    public static int packedLength(int texels) {
        return (texels + 1) / 2;
    }

    /** Palette index of texel {@code i} of a packed face (0 past the end). */
    public static int texel(byte[] face, int i) {
        int b = i >> 1;
        if (b >= face.length) return 0;
        return (i & 1) == 0 ? (face[b] >> 4) & 0xF : face[b] & 0xF;
    }

    public static void setTexel(byte[] face, int i, int index) {
        int b = i >> 1;
        if (b >= face.length) return;
        if ((i & 1) == 0) face[b] = (byte) ((face[b] & 0x0F) | (index & 0xF) << 4);
        else face[b] = (byte) ((face[b] & 0xF0) | (index & 0xF));
    }

    /** The ARGB colour of texel {@code i} of a face (opaque). */
    public int argb(boolean frontFace, int i) {
        int idx = texel(frontFace ? front : back, i);
        return 0xFF000000 | palette[Math.min(idx, palette.length - 1)];
    }

    /** Both faces sized for {@code kind}? */
    public boolean fits(CardKind kind) {
        int n = packedLength(kind.texels());
        return front.length == n && back.length == n;
    }

    /** Blank faces for a kind: palette {white}, every texel 0. */
    public static CardFaces blank(CardKind kind) {
        int n = packedLength(kind.texels());
        return new CardFaces(new int[] {0xF4F1EA}, new byte[n], new byte[n]);
    }

    /** Stable hash for texture caching (content, not identity). */
    public long contentHash() {
        long h = 1469598103934665603L;
        for (int c : palette) h = (h ^ c) * 1099511628211L;
        for (byte b : front) h = (h ^ (b & 0xff)) * 1099511628211L;
        h = (h ^ 0x5A) * 1099511628211L;
        for (byte b : back) h = (h ^ (b & 0xff)) * 1099511628211L;
        return h;
    }

    @Override
    public boolean equals(Object o) {
        return o instanceof CardFaces f && Arrays.equals(palette, f.palette) && Arrays.equals(front, f.front) && Arrays.equals(back, f.back);
    }

    @Override
    public int hashCode() {
        return Long.hashCode(contentHash());
    }

    @Override
    public String toString() {
        return "CardFaces[" + palette.length + " colours, " + front.length + "+" + back.length + " bytes]";
    }

    private static final Codec<byte[]> BYTES = Codec.BYTE_BUFFER.xmap(b -> {
        byte[] out = new byte[b.remaining()];
        b.duplicate().get(out);
        return out;
    }, java.nio.ByteBuffer::wrap);

    public static final Codec<CardFaces> CODEC = RecordCodecBuilder.<CardFaces>create(i -> i.group(
            Codec.INT.listOf().fieldOf("palette").forGetter(f -> Arrays.stream(f.palette).boxed().toList()),
            BYTES.fieldOf("front").forGetter(CardFaces::front),
            BYTES.fieldOf("back").forGetter(CardFaces::back)
    ).apply(i, (p, f, b) -> new CardFaces(p.stream().mapToInt(Integer::intValue).toArray(), f, b))).flatXmap(DataResult::success, f -> {
        try {
            return DataResult.success(f);
        } catch (IllegalArgumentException e) {
            return DataResult.error(e::getMessage);
        }
    });

    public static final StreamCodec<ByteBuf, CardFaces> STREAM_CODEC = StreamCodec.composite(
            ByteBufCodecs.INT.apply(ByteBufCodecs.list(MAX_PALETTE)), f -> {
                List<Integer> l = new ArrayList<>();
                for (int c : f.palette) l.add(c);
                return l;
            },
            ByteBufCodecs.BYTE_ARRAY, CardFaces::front,
            ByteBufCodecs.BYTE_ARRAY, CardFaces::back,
            (p, f, b) -> new CardFaces(p.stream().mapToInt(Integer::intValue).toArray(), f, b));

    public static net.minecraft.core.component.DataComponentType<CardFaces> componentType() {
        return net.minecraft.core.component.DataComponentType.<CardFaces>builder().persistent(CODEC).networkSynchronized(STREAM_CODEC).build();
    }
}
