package com.example.customworld.block;

import com.example.customworld.CustomWorldMod;
import net.minecraft.world.item.CreativeModeTabs;
import net.minecraftforge.event.BuildCreativeModeTabContentsEvent;
import net.minecraftforge.eventbus.api.SubscribeEvent;
import net.minecraftforge.fml.common.Mod;

/**
 * Handles adding mod items to creative mode tabs.
 */
@Mod.EventBusSubscriber(modid = CustomWorldMod.MODID, bus = Mod.EventBusSubscriber.Bus.MOD)
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
