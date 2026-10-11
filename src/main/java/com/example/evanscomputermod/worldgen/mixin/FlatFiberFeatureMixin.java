package com.example.evanscomputermod.worldgen.mixin;

//? if <=1.21.1 {
import com.example.evanscomputermod.worldgen.FiberLineFeature;
import com.example.evanscomputermod.worldgen.TechWorldgen;
import net.minecraft.core.Holder;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.BiomeGenerationSettings;
import net.minecraft.world.level.levelgen.GenerationStep;
import net.minecraft.world.level.levelgen.flat.FlatLevelGeneratorSettings;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

/**
 * Superflat generators drop biome features unless the preset enables decoration
 * (vanilla's default preset does not), which would also drop the biome-modifier fiber.
 * Re-add the fiber feature at the last decoration step so superflat worlds get the
 * same long-distance fiber as normal worlds.
 */
@Mixin(FlatLevelGeneratorSettings.class)
public abstract class FlatFiberFeatureMixin {
    @Inject(method = "adjustGenerationSettings", at = @At("RETURN"), cancellable = true)
    private void ecm$addFiber(Holder<Biome> biome, CallbackInfoReturnable<BiomeGenerationSettings> cir) {
        BiomeGenerationSettings settings = cir.getReturnValue();
        var steps = settings.features();
        for (var step : steps)
            for (var placed : step)
                if (placed.value().feature().value().feature() == TechWorldgen.FIBER_LINE.get()) return;
        var builder = new BiomeGenerationSettings.PlainBuilder();
        for (int i = 0; i < steps.size(); i++)
            for (var placed : steps.get(i)) builder.addFeature(i, placed);
        for (var carving : GenerationStep.Carving.values())
            for (var carver : settings.getCarvers(carving)) builder.addCarver(carving, carver);
        builder.addFeature(GenerationStep.Decoration.TOP_LAYER_MODIFICATION, FiberLineFeature.inlinePlaced());
        cir.setReturnValue(builder.build());
    }
}
//?}
