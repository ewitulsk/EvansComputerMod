package com.example.evanscomputermod.block;

import com.example.evanscomputermod.EvansComputerMod;
import net.minecraft.core.registries.Registries;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredRegister;

/**
 * Registry for all block entities in the mod.
 */
public class ModBlockEntities {
    
    public static final DeferredRegister<BlockEntityType<?>> BLOCK_ENTITIES =
            DeferredRegister.create(Registries.BLOCK_ENTITY_TYPE, EvansComputerMod.MODID);
    
    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<TerminalBlockEntity>> TERMINAL_BLOCK_ENTITY =
            BLOCK_ENTITIES.register("terminal_block_entity", () ->
                    new BlockEntityType<>(TerminalBlockEntity::new, ModBlocks.TERMINAL_BLOCK.get())
            );
}
