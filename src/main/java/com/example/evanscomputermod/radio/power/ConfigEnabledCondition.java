package com.example.evanscomputermod.radio.power;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.RadioConfig;
import com.mojang.serialization.Codec;
import com.mojang.serialization.MapCodec;
import com.mojang.serialization.codecs.RecordCodecBuilder;
import net.neoforged.neoforge.common.conditions.ICondition;

/**
 * Recipe condition {@code evanscomputermod:radio_feature_enabled} — true when the
 * named optional radio block is enabled in the server config, so packs can turn
 * a block off without unregistering it (e.g. {@code "feature": "burner_generator"}).
 */
public record ConfigEnabledCondition(String feature) implements ICondition {

    public static final MapCodec<ConfigEnabledCondition> CODEC = RecordCodecBuilder.mapCodec(i -> i.group(
            Codec.STRING.fieldOf("feature").forGetter(ConfigEnabledCondition::feature)
    ).apply(i, ConfigEnabledCondition::new));

    @Override
    public boolean test(IContext context) {
        return switch (feature) {
            case "burner_generator" -> RadioConfig.burnerGeneratorEnabled();
            default -> true;
        };
    }

    @Override
    public MapCodec<? extends ICondition> codec() {
        return CODEC;
    }
}
//?}
