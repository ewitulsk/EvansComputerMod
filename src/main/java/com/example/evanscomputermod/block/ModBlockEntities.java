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
                    //? if >=26.1 {
                    new BlockEntityType<>(TerminalBlockEntity::new, ModBlocks.TERMINAL_BLOCK.get())
                    //?} else
                    /*BlockEntityType.Builder.<TerminalBlockEntity>of(TerminalBlockEntity::new, ModBlocks.TERMINAL_BLOCK.get()).build(null)*/
            );

    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<ScreenBlockEntity>> SCREEN_BLOCK_ENTITY =
            BLOCK_ENTITIES.register("screen_block_entity", () ->
                    //? if >=26.1 {
                    new BlockEntityType<>(ScreenBlockEntity::new, ModBlocks.SCREEN_BLOCK.get())
                    //?} else
                    /*BlockEntityType.Builder.<ScreenBlockEntity>of(ScreenBlockEntity::new, ModBlocks.SCREEN_BLOCK.get()).build(null)*/
            );

    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<MissileLauncherBlockEntity>> MISSILE_LAUNCHER_BLOCK_ENTITY =
            BLOCK_ENTITIES.register("missile_launcher_block_entity", () ->
                    //? if >=26.1 {
                    new BlockEntityType<>(MissileLauncherBlockEntity::new, ModBlocks.MISSILE_LAUNCHER.get())
                    //?} else
                    /*BlockEntityType.Builder.<MissileLauncherBlockEntity>of(MissileLauncherBlockEntity::new, ModBlocks.MISSILE_LAUNCHER.get()).build(null)*/
            );
}
