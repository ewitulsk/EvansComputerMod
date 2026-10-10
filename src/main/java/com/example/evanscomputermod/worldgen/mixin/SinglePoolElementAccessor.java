package com.example.evanscomputermod.worldgen.mixin;

//? if <=1.21.1 {
import com.mojang.datafixers.util.Either;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.level.levelgen.structure.pools.SinglePoolElement;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplate;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

/** The template location of a placed village piece (houses, streets, the ISP). */
@Mixin(SinglePoolElement.class)
public interface SinglePoolElementAccessor {
    @Accessor("template")
    Either<ResourceLocation, StructureTemplate> ecm$template();
}
//?}
