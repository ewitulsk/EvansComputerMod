package com.example.evanscomputermod.worldgen.mixin;

//? if <=1.21.1 {
import java.util.stream.Stream;
import net.minecraft.core.Holder;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.ResourceKey;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.level.chunk.ChunkGenerator;
import net.minecraft.world.level.chunk.ChunkGeneratorStructureState;
import net.minecraft.world.level.levelgen.FlatLevelSource;
import net.minecraft.world.level.levelgen.flat.FlatLevelGeneratorSettings;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.structure.StructureSet;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

/**
 * Superflat worlds generate only the structure sets their preset lists
 * (vanilla's default: strongholds and villages), so Tech Villages never
 * spawned there. Add the Tech Village set to any flat world that has such a
 * list; a flat world without one already generates every set.
 */
@Mixin(FlatLevelSource.class)
public abstract class FlatTechVillageMixin {
    private static final ResourceKey<StructureSet> ECM_TECH_VILLAGES = ResourceKey.create(
            Registries.STRUCTURE_SET, ResourceLocation.fromNamespaceAndPath("evanscomputermod", "tech_villages"));

    @Shadow @Final private FlatLevelGeneratorSettings settings;

    @Inject(method = "createState", at = @At("HEAD"), cancellable = true)
    private void ecm$addTechVillages(HolderLookup<StructureSet> lookup, RandomState randomState, long seed,
                                     CallbackInfoReturnable<ChunkGeneratorStructureState> cir) {
        var overrides = settings.structureOverrides();
        if (overrides.isEmpty()) return;
        var tech = lookup.get(ECM_TECH_VILLAGES);
        if (tech.isEmpty() || overrides.get().contains(tech.get())) return;
        Stream<Holder<StructureSet>> sets = Stream.concat(overrides.get().stream(), Stream.<Holder<StructureSet>>of(tech.get()));
        cir.setReturnValue(ChunkGeneratorStructureState.createForFlat(
                randomState, seed, ((ChunkGenerator) (Object) this).getBiomeSource(), sets));
    }
}
//?}
