package com.example.customworld.block;

import com.example.customworld.CustomWorldMod;
import net.minecraft.world.item.CreativeModeTabs;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.event.BuildCreativeModeTabContentsEvent;

/**
 * Handles adding mod items to creative mode tabs.
 */
@EventBusSubscriber(modid = CustomWorldMod.MODID, bus = EventBusSubscriber.Bus.MOD)
public class ModCreativeTabs {
    
    @SubscribeEvent
    public static void addCreative(BuildCreativeModeTabContentsEvent event) {
        // Add terminal block to the Redstone tab (fits the "computer" theme)
        if (event.getTabKey() == CreativeModeTabs.REDSTONE_BLOCKS) {
            event.accept(ModBlocks.TERMINAL_BLOCK_ITEM.get());
        }
        
        // Also add to Functional Blocks tab
        if (event.getTabKey() == CreativeModeTabs.FUNCTIONAL_BLOCKS) {
            event.accept(ModBlocks.TERMINAL_BLOCK_ITEM.get());
        }
    }
}
