package com.example.evanscomputermod.card;

import com.mojang.serialization.Codec;
import com.mojang.serialization.codecs.RecordCodecBuilder;
import io.netty.buffer.ByteBuf;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;

import java.util.Random;

/**
 * A magnetic stripe ({@code evanscomputermod:card_stripe}): three tracks of plain
 * printable ASCII (64 / 32 / 96 characters at most), no authentication, no secret.
 * Public and synced. {@code corrupted} is set once a strong field scrambled it
 * ({@link StripeHazard}); writing the stripe clears it.
 */
public record Stripe(String t1, String t2, String t3, boolean corrupted) {
    public static final int T1_MAX = 64, T2_MAX = 32, T3_MAX = 96;
    public static final Stripe EMPTY = new Stripe("", "", "", false);

    public Stripe {
        t1 = check(t1, T1_MAX, 1);
        t2 = check(t2, T2_MAX, 2);
        t3 = check(t3, T3_MAX, 3);
    }

    private static String check(String s, int max, int track) {
        if (s == null) s = "";
        if (s.length() > max) throw new IllegalArgumentException("track " + track + " holds at most " + max + " characters");
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            if (c < 0x20 || c > 0x7E) throw new IllegalArgumentException("track " + track + " is printable ASCII only");
        }
        return s;
    }

    public boolean isEmpty() {
        return t1.isEmpty() && t2.isEmpty() && t3.isEmpty();
    }

    /** The stripe after a strong field: random printable characters at random positions of every track. */
    public Stripe corrupt(Random rnd) {
        return new Stripe(scramble(t1, rnd), scramble(t2, rnd), scramble(t3, rnd), true);
    }

    private static String scramble(String s, Random rnd) {
        if (s.isEmpty()) return s;
        char[] c = s.toCharArray();
        int n = Math.max(1, c.length / 3);
        for (int i = 0; i < n; i++) c[rnd.nextInt(c.length)] = (char) (0x21 + rnd.nextInt(0x7E - 0x21));
        return new String(c);
    }

    public static final Codec<Stripe> CODEC = RecordCodecBuilder.create(i -> i.group(
            Codec.STRING.fieldOf("t1").forGetter(Stripe::t1),
            Codec.STRING.fieldOf("t2").forGetter(Stripe::t2),
            Codec.STRING.fieldOf("t3").forGetter(Stripe::t3),
            Codec.BOOL.optionalFieldOf("corrupted", false).forGetter(Stripe::corrupted)
    ).apply(i, Stripe::new));

    public static final StreamCodec<ByteBuf, Stripe> STREAM_CODEC = StreamCodec.composite(
            ByteBufCodecs.STRING_UTF8, Stripe::t1,
            ByteBufCodecs.STRING_UTF8, Stripe::t2,
            ByteBufCodecs.STRING_UTF8, Stripe::t3,
            ByteBufCodecs.BOOL, Stripe::corrupted,
            Stripe::new);

    public static net.minecraft.core.component.DataComponentType<Stripe> componentType() {
        return net.minecraft.core.component.DataComponentType.<Stripe>builder().persistent(CODEC).networkSynchronized(STREAM_CODEC).build();
    }
}
