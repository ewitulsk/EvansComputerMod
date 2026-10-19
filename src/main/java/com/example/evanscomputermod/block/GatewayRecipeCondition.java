package com.example.evanscomputermod.block;

//? if <=1.21.1 {
import com.example.evanscomputermod.EcmConfig;
import com.mojang.serialization.MapCodec;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.common.conditions.ICondition;
import net.neoforged.neoforge.registries.DeferredRegister;
import net.neoforged.neoforge.registries.NeoForgeRegistries;

/**
 * Recipe condition {@code evanscomputermod:internet_gateway_recipe_enabled}: true when
 * {@code internet.gatewayRecipe} is on in {@code config/evanscomputermod-common.toml}. That
 * file is loaded at startup, before any datapack, so the condition sees it on a world's
 * first load; a change takes effect at the next {@code /reload}.
 */
public record GatewayRecipeCondition() implements ICondition {

    public static final GatewayRecipeCondition INSTANCE = new GatewayRecipeCondition();
    public static final MapCodec<GatewayRecipeCondition> CODEC = MapCodec.unit(INSTANCE);

    private static final DeferredRegister<MapCodec<? extends ICondition>> CONDITIONS =
            DeferredRegister.create(NeoForgeRegistries.Keys.CONDITION_CODECS, com.example.evanscomputermod.EvansComputerMod.MODID);

    static {
        CONDITIONS.register("internet_gateway_recipe_enabled", () -> CODEC);
    }

    public static void register(IEventBus modBus) {
        CONDITIONS.register(modBus);
    }

    @Override
    public boolean test(IContext context) {
        return EcmConfig.internetGatewayRecipe();
    }

    @Override
    public MapCodec<? extends ICondition> codec() {
        return CODEC;
    }
}
//?}
