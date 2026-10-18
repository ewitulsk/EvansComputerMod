package com.example.evanscomputermod.card;

import com.mojang.serialization.Codec;
import com.mojang.serialization.DataResult;
import com.mojang.serialization.codecs.RecordCodecBuilder;
import io.netty.buffer.ByteBuf;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;

/**
 * The public half of a written SIM card: its carrier, subscriber identity (IMSI)
 * and phone number ({@code msisdn}, E.164, empty when the carrier gave it none:
 * docs/TELEPHONY.md). Stored on the SIM item as the {@code evanscomputermod:sim_card}
 * component, saved and synced to clients (tooltips show it). A blank card has none.
 * The secret key is a separate component, {@link SimSecret}. The network's copy of
 * the number (the HSS) wins when they differ; the card's lets the tooltip and
 * Settings show it before the phone has attached.
 */
public record SimIdentity(String carrier, long imsi, String msisdn) {
    public SimIdentity {
        carrier = SimAuth.parseCarrier(carrier);
        if (imsi < 0 || imsi > 999_999_999_999_999L) throw new IllegalArgumentException("bad IMSI " + imsi);
        msisdn = SimAuth.parseMsisdn(msisdn);
    }

    /** A card without a number. */
    public SimIdentity(String carrier, long imsi) {
        this(carrier, imsi, "");
    }

    /** The IMSI as the 15 digits phones show. */
    public String imsiText() {
        return SimAuth.formatImsi(imsi);
    }

    /** {@code (214) 555-0142} for a +1 number, the E.164 form otherwise ("" without one). */
    public String numberText() {
        if (msisdn.startsWith("+1") && msisdn.length() == 12) {
            return "(" + msisdn.substring(2, 5) + ") " + msisdn.substring(5, 8) + "-" + msisdn.substring(8);
        }
        return msisdn;
    }

    private record Raw(String carrier, long imsi, String msisdn) {}

    public static final Codec<SimIdentity> CODEC = RecordCodecBuilder.<Raw>create(i -> i.group(
            Codec.STRING.fieldOf("carrier").forGetter(Raw::carrier),
            Codec.LONG.fieldOf("imsi").forGetter(Raw::imsi),
            Codec.STRING.optionalFieldOf("msisdn", "").forGetter(Raw::msisdn)
    ).apply(i, Raw::new)).flatXmap(r -> {
        try {
            return DataResult.success(new SimIdentity(r.carrier(), r.imsi(), r.msisdn()));
        } catch (IllegalArgumentException e) {
            return DataResult.error(() -> "bad SIM identity: " + e.getMessage());
        }
    }, s -> DataResult.success(new Raw(s.carrier(), s.imsi(), s.msisdn())));

    public static final StreamCodec<ByteBuf, SimIdentity> STREAM_CODEC = StreamCodec.composite(
            ByteBufCodecs.STRING_UTF8, SimIdentity::carrier,
            ByteBufCodecs.VAR_LONG, SimIdentity::imsi,
            ByteBufCodecs.STRING_UTF8, SimIdentity::msisdn,
            SimIdentity::new);

    /** The data component type (registered by CellularContent; built here so tests use the same one). */
    public static net.minecraft.core.component.DataComponentType<SimIdentity> componentType() {
        return net.minecraft.core.component.DataComponentType.<SimIdentity>builder().persistent(CODEC).networkSynchronized(STREAM_CODEC).build();
    }
}
