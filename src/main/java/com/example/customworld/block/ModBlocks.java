package com.example.customworld.block;

import com.example.customworld.CustomWorldMod;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraftforge.registries.DeferredRegister;
import net.minecraftforge.registries.ForgeRegistries;
import net.minecraftforge.registries.RegistryObject;

/**
 * Registry for all blocks in the mod.
 */
public class ModBlocks {

    public static final DeferredRegister<Block> BLOCKS =
            DeferredRegister.create(ForgeRegistries.BLOCKS, CustomWorldMod.MODID);

    public static final DeferredRegister<Item> BLOCK_ITEMS =
            DeferredRegister.create(ForgeRegistries.ITEMS, CustomWorldMod.MODID);

    // Terminal Block - uses gold block properties for similar feel
    public static final RegistryObject<TerminalBlock> TERMINAL_BLOCK =
            BLOCKS.register("terminal_block", () -> new TerminalBlock(
                    BlockBehaviour.Properties.of()
                            .strength(3.0f, 6.0f)
                            .sound(SoundType.METAL)
                            .requiresCorrectToolForDrops()
            ));

    // Block item for the terminal
    public static final RegistryObject<BlockItem> TERMINAL_BLOCK_ITEM =
            BLOCK_ITEMS.register("terminal_block", () -> new BlockItem(
                    TERMINAL_BLOCK.get(),
                    new Item.Properties()
            ));
}
