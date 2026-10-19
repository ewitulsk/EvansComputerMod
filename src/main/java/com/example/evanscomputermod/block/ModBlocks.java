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
    public static final DeferredBlock<Block> FIBER_SPAN=BLOCKS.registerBlock("fiber_span",FiberInfrastructureBlock::new,BlockBehaviour.Properties.of().strength(0.5f).noOcclusion().forceSolidOn().sound(SoundType.METAL));
    public static final DeferredBlock<FiberPatchPanelBlock> FIBER_PATCH_PANEL=BLOCKS.registerBlock("fiber_patch_panel",FiberPatchPanelBlock::new,BlockBehaviour.Properties.of().strength(1).forceSolidOn().sound(SoundType.METAL));
    public static final DeferredItem<BlockItem> FIBER_SPAN_ITEM=BLOCK_ITEMS.registerSimpleBlockItem("fiber_span",FIBER_SPAN);
    public static final DeferredItem<BlockItem> FIBER_PATCH_PANEL_ITEM=BLOCK_ITEMS.registerSimpleBlockItem("fiber_patch_panel",FIBER_PATCH_PANEL);

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
                            // Water must not wash cables away: a thin non-solid block is
                            // replaced by flowing fluid, and buried village cables run
                            // beside rivers and the sea.
                            .forceSolidOn()
            );

    public static final DeferredItem<BlockItem> NETWORK_CABLE_ITEM =
            BLOCK_ITEMS.registerSimpleBlockItem("network_cable", NETWORK_CABLE);

    // Internet Gateway - the uplink to the host's network. The one generated at (0,0,0) is
    // unbreakable; player-placed ones (InternetGatewayBlock.CRAFTED) break with a pickaxe.
    public static final DeferredBlock<InternetGatewayBlock> INTERNET_GATEWAY =
            BLOCKS.registerBlock("internet_gateway", InternetGatewayBlock::new,
                    BlockBehaviour.Properties.of()
                            .strength(5.0f, 3600000.0f)
                            .requiresCorrectToolForDrops()
                            .sound(SoundType.METAL)
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

    /** A Terminal item with {@code panel} installed (what the recipe crafts with a Tier 1 panel). */
    public static net.minecraft.world.item.ItemStack terminalWithPanel(net.minecraft.world.item.Item panel) {
        var stack = new net.minecraft.world.item.ItemStack(TERMINAL_BLOCK_ITEM.get());
        stack.set(com.example.evanscomputermod.module.ModDataComponents.INSTALLED_PANEL.get(),
                new com.example.evanscomputermod.module.InstalledPanel(new net.minecraft.world.item.ItemStack(panel)));
        return stack;
    }

    // Tiered Screens: screen_block above is Tier 2 (Color LCD), as it was
    // before tiers. A wall runs at its lowest block's tier.
    public static final DeferredBlock<ScreenBlock> PHOSPHOR_SCREEN =
            BLOCKS.registerBlock("phosphor_screen",
                    p -> new ScreenBlock(p, com.example.evanscomputermod.computer.display.PanelTier.PHOSPHOR),
                    BlockBehaviour.Properties.of().strength(1.0f, 2.0f).sound(SoundType.GLASS).lightLevel(state -> 4));
    public static final DeferredItem<BlockItem> PHOSPHOR_SCREEN_ITEM =
            BLOCK_ITEMS.registerSimpleBlockItem("phosphor_screen", PHOSPHOR_SCREEN);
    public static final DeferredBlock<ScreenBlock> HD_SCREEN =
            BLOCKS.registerBlock("hd_screen",
                    p -> new ScreenBlock(p, com.example.evanscomputermod.computer.display.PanelTier.HD),
                    BlockBehaviour.Properties.of().strength(1.0f, 2.0f).sound(SoundType.GLASS).lightLevel(state -> 8));
    public static final DeferredItem<BlockItem> HD_SCREEN_ITEM =
            BLOCK_ITEMS.registerSimpleBlockItem("hd_screen", HD_SCREEN);
}
