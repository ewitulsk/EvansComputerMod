package com.example.customworld.block;

import com.example.customworld.CustomWorldMod;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraftforge.registries.DeferredRegister;
import net.minecraftforge.registries.ForgeRegistries;
import net.minecraftforge.registries.RegistryObject;

/**
 * Registry for all block entities in the mod.
 */
public class ModBlockEntities {

    public static final DeferredRegister<BlockEntityType<?>> BLOCK_ENTITIES =
            DeferredRegister.create(ForgeRegistries.BLOCK_ENTITY_TYPES, CustomWorldMod.MODID);

    public static final RegistryObject<BlockEntityType<TerminalBlockEntity>> TERMINAL_BLOCK_ENTITY =
            BLOCK_ENTITIES.register("terminal_block_entity", () ->
                    BlockEntityType.Builder.of(TerminalBlockEntity::new, ModBlocks.TERMINAL_BLOCK.get())
                            .build(null)
            );
}
