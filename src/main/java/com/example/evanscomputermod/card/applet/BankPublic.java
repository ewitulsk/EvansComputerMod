package com.example.evanscomputermod.card.applet;

import com.mojang.serialization.Codec;
import com.mojang.serialization.DataResult;
import com.mojang.serialization.codecs.RecordCodecBuilder;
import io.netty.buffer.ByteBuf;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;

import java.util.Locale;
import java.util.regex.Pattern;

/**
 * The public half of a written bank chip ({@code evanscomputermod:bank_applet}): issuer,
 * PAN, whether a PIN is required, the monotonic signing counter, failed PIN tries and
 * whether the chip is locked. Saved and synced (tooltips). The key and PIN hash are
 * {@link BankSecret}.
 */
public record BankPublic(String issuer, String pan, boolean pinRequired, long ctr, int pinFails, boolean locked) {
    public static final Pattern ISSUER = Pattern.compile("[a-z0-9_-]{1,24}");
    public static final Pattern PAN = Pattern.compile("[0-9]{8,19}");
    public static final int MAX_PIN_FAILS = 3;

    public BankPublic {
        if (issuer == null || !ISSUER.matcher(issuer).matches()) throw new IllegalArgumentException("bad issuer id: " + issuer);
        if (pan == null || !PAN.matcher(pan).matches()) throw new IllegalArgumentException("a PAN is 8-19 digits");
        if (ctr < 0) throw new IllegalArgumentException("bad counter");
    }

    public BankPublic withCtr(long c) {
        return new BankPublic(issuer, pan, pinRequired, c, pinFails, locked);
    }

    public BankPublic withFails(int f) {
        return new BankPublic(issuer, pan, pinRequired, ctr, f, f >= MAX_PIN_FAILS || locked);
    }

    /** The PAN with a space every four digits, as cards print it. */
    public String panText() {
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < pan.length(); i++) {
            if (i > 0 && i % 4 == 0) sb.append(' ');
            sb.append(pan.charAt(i));
        }
        return sb.toString();
    }

    private record Raw(String issuer, String pan, boolean pinRequired, long ctr, int pinFails, boolean locked) {}

    public static final Codec<BankPublic> CODEC = RecordCodecBuilder.<Raw>create(i -> i.group(
            Codec.STRING.fieldOf("issuer").forGetter(Raw::issuer),
            Codec.STRING.fieldOf("pan").forGetter(Raw::pan),
            Codec.BOOL.optionalFieldOf("pin_required", false).forGetter(Raw::pinRequired),
            Codec.LONG.optionalFieldOf("ctr", 0L).forGetter(Raw::ctr),
            Codec.INT.optionalFieldOf("pin_fails", 0).forGetter(Raw::pinFails),
            Codec.BOOL.optionalFieldOf("locked", false).forGetter(Raw::locked)
    ).apply(i, Raw::new)).flatXmap(r -> {
        try {
            return DataResult.success(new BankPublic(r.issuer().toLowerCase(Locale.ROOT), r.pan(), r.pinRequired(), r.ctr(), r.pinFails(), r.locked()));
        } catch (IllegalArgumentException e) {
            return DataResult.error(() -> "bad bank applet: " + e.getMessage());
        }
    }, b -> DataResult.success(new Raw(b.issuer(), b.pan(), b.pinRequired(), b.ctr(), b.pinFails(), b.locked())));

    public static final StreamCodec<ByteBuf, BankPublic> STREAM_CODEC = StreamCodec.composite(
            ByteBufCodecs.STRING_UTF8, BankPublic::issuer,
            ByteBufCodecs.STRING_UTF8, BankPublic::pan,
            ByteBufCodecs.BOOL, BankPublic::pinRequired,
            ByteBufCodecs.VAR_LONG, BankPublic::ctr,
            ByteBufCodecs.VAR_INT, BankPublic::pinFails,
            ByteBufCodecs.BOOL, BankPublic::locked,
            BankPublic::new);

    public static net.minecraft.core.component.DataComponentType<BankPublic> componentType() {
        return net.minecraft.core.component.DataComponentType.<BankPublic>builder().persistent(CODEC).networkSynchronized(STREAM_CODEC).build();
    }
}
