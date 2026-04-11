package com.example.evanscomputermod.block;

import com.example.evanscomputermod.EvansComputerMod;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.neoforged.neoforge.registries.DeferredBlock;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;

/**
 * Registry for all blocks in the mod.
 */
public class ModBlocks {

    public static final DeferredRegister.Blocks BLOCKS =
            DeferredRegister.createBlocks(EvansComputerMod.MODID);

    public static final DeferredRegister.Items BLOCK_ITEMS =
            DeferredRegister.createItems(EvansComputerMod.MODID);

    // Terminal Block - uses gold block properties for similar feel
    public static final DeferredBlock<TerminalBlock> TERMINAL_BLOCK =
            BLOCKS.registerBlock("terminal_block", TerminalBlock::new,
                    BlockBehaviour.Properties.of()
                            .strength(3.0f, 6.0f)
                            .sound(SoundType.METAL)
                            .requiresCorrectToolForDrops()
            );

    // Block item for the terminal
    public static final DeferredItem<BlockItem> TERMINAL_BLOCK_ITEM =
            BLOCK_ITEMS.registerSimpleBlockItem("terminal_block", TERMINAL_BLOCK);

    // Network Cable - connects computers together
    public static final DeferredBlock<NetworkCableBlock> NETWORK_CABLE =
            BLOCKS.registerBlock("network_cable", NetworkCableBlock::new,
                    BlockBehaviour.Properties.of()
                            .strength(0.5f, 0.5f)
                            .sound(SoundType.WOOL)
                            .noOcclusion()
            );

    public static final DeferredItem<BlockItem> NETWORK_CABLE_ITEM =
            BLOCK_ITEMS.registerSimpleBlockItem("network_cable", NETWORK_CABLE);

    // Internet Gateway - unbreakable block at (0,0,0) for TAP bridge access
    public static final DeferredBlock<InternetGatewayBlock> INTERNET_GATEWAY =
            BLOCKS.registerBlock("internet_gateway", InternetGatewayBlock::new,
                    BlockBehaviour.Properties.of()
                            .strength(-1.0f, 3600000.0f)
                            .sound(SoundType.METAL)
                            .noLootTable()
                            .pushReaction(net.minecraft.world.level.material.PushReaction.BLOCK)
            );

    public static final DeferredItem<BlockItem> INTERNET_GATEWAY_ITEM =
            BLOCK_ITEMS.registerSimpleBlockItem("internet_gateway", INTERNET_GATEWAY);

    // Network Interface - connects to cables/terminals and provides additional interfaces
    public static final DeferredBlock<InterfaceBlock> INTERFACE_BLOCK =
            BLOCKS.registerBlock("interface_block", InterfaceBlock::new,
                    BlockBehaviour.Properties.of()
                            .strength(0.5f, 0.5f)
                            .sound(SoundType.METAL)
                            .noOcclusion()
            );

    public static final DeferredItem<BlockItem> INTERFACE_BLOCK_ITEM =
            BLOCK_ITEMS.registerSimpleBlockItem("interface_block", INTERFACE_BLOCK);

    // Screen Block - in-world display for a connected computer
    public static final DeferredBlock<ScreenBlock> SCREEN_BLOCK =
            BLOCKS.registerBlock("screen_block", ScreenBlock::new,
                    BlockBehaviour.Properties.of()
                            .strength(1.0f, 2.0f)
                            .sound(SoundType.GLASS)
                            .lightLevel(state -> 6)
            );

    public static final DeferredItem<BlockItem> SCREEN_BLOCK_ITEM =
            BLOCK_ITEMS.registerSimpleBlockItem("screen_block", SCREEN_BLOCK);
}
