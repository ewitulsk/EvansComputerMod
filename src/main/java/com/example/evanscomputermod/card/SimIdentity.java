package com.example.evanscomputermod.card;

import com.mojang.serialization.Codec;
import com.mojang.serialization.DataResult;
import com.mojang.serialization.codecs.RecordCodecBuilder;
import io.netty.buffer.ByteBuf;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;

/**
 * The public half of a written SIM card: its carrier and subscriber number.
 * Stored on the SIM item as the {@code evanscomputermod:sim_card} component,
 * saved and synced to clients (tooltips show it). A blank card has none.
 * The secret key is a separate component, {@link SimSecret}.
 */
public record SimIdentity(String carrier, long imsi) {
    public SimIdentity {
        carrier = SimAuth.parseCarrier(carrier);
        if (imsi < 0 || imsi > 999_999_999_999_999L) throw new IllegalArgumentException("bad IMSI " + imsi);
    }

    /** The IMSI as the 15 digits phones show. */
    public String imsiText() {
        return SimAuth.formatImsi(imsi);
    }

    private record Raw(String carrier, long imsi) {}

    public static final Codec<SimIdentity> CODEC = RecordCodecBuilder.<Raw>create(i -> i.group(
            Codec.STRING.fieldOf("carrier").forGetter(Raw::carrier),
            Codec.LONG.fieldOf("imsi").forGetter(Raw::imsi)
    ).apply(i, Raw::new)).flatXmap(r -> {
        try {
            return DataResult.success(new SimIdentity(r.carrier(), r.imsi()));
        } catch (IllegalArgumentException e) {
            return DataResult.error(() -> "bad SIM identity: " + e.getMessage());
        }
    }, s -> DataResult.success(new Raw(s.carrier(), s.imsi())));

    public static final StreamCodec<ByteBuf, SimIdentity> STREAM_CODEC = StreamCodec.composite(
            ByteBufCodecs.STRING_UTF8, SimIdentity::carrier,
            ByteBufCodecs.VAR_LONG, SimIdentity::imsi,
            SimIdentity::new);

    /** The data component type (registered by CellularContent; built here so tests use the same one). */
    public static net.minecraft.core.component.DataComponentType<SimIdentity> componentType() {
        return net.minecraft.core.component.DataComponentType.<SimIdentity>builder().persistent(CODEC).networkSynchronized(STREAM_CODEC).build();
    }
}
