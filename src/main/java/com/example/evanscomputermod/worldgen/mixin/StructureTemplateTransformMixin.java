package com.example.evanscomputermod.worldgen.mixin;

//? if <=1.21.1 {
import com.example.evanscomputermod.worldgen.StructureTransformable;
import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.llamalad7.mixinextras.sugar.Local;
import net.minecraft.core.HolderLookup;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructurePlaceSettings;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplate;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

/**
 * Structure placement mirrors and rotates each block state but loads the block entity's
 * NBT as saved. Block entities that store world directions ({@link StructureTransformable})
 * are turned by the same mirror and rotation right after that load.
 */
@Mixin(StructureTemplate.class)
public abstract class StructureTemplateTransformMixin {
    @WrapOperation(method = "placeInWorld", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/world/level/block/entity/BlockEntity;loadWithComponents(Lnet/minecraft/nbt/CompoundTag;Lnet/minecraft/core/HolderLookup$Provider;)V"))
    private void ecm$transformBlockEntity(BlockEntity be, CompoundTag tag, HolderLookup.Provider registries,
                                          Operation<Void> original, @Local(argsOnly = true) StructurePlaceSettings settings) {
        original.call(be, tag, registries);
        if (be instanceof StructureTransformable t) t.transformForStructure(settings.getMirror(), settings.getRotation());
    }
}
//?}
