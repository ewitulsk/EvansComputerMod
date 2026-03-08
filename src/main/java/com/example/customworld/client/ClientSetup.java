package com.example.customworld.client;

import com.example.customworld.CustomWorldMod;
import com.example.customworld.block.ModMenuTypes;
import com.example.customworld.block.TerminalScreen;
import net.minecraft.client.gui.screens.MenuScreens;
import net.minecraftforge.api.distmarker.Dist;
import net.minecraftforge.eventbus.api.SubscribeEvent;
import net.minecraftforge.fml.common.Mod;
import net.minecraftforge.fml.event.lifecycle.FMLClientSetupEvent;

/**
 * Client-side setup and event handlers.
 */
@Mod.EventBusSubscriber(modid = CustomWorldMod.MODID, bus = Mod.EventBusSubscriber.Bus.MOD, value = Dist.CLIENT)
public class ClientSetup {

    @SubscribeEvent
    public static void onClientSetup(FMLClientSetupEvent event) {
        event.enqueueWork(() -> {
            MenuScreens.register(ModMenuTypes.TERMINAL_MENU.get(), TerminalScreen::new);
        });
        CustomWorldMod.LOGGER.info("Registered terminal screen");
    }
}
