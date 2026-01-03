package com.example.customworld.client;

import com.example.customworld.CustomWorldMod;
import com.example.customworld.block.ModMenuTypes;
import com.example.customworld.block.TerminalScreen;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.client.event.RegisterMenuScreensEvent;

/**
 * Client-side setup and event handlers.
 */
@EventBusSubscriber(modid = CustomWorldMod.MODID, bus = EventBusSubscriber.Bus.MOD, value = Dist.CLIENT)
public class ClientSetup {
    
    @SubscribeEvent
    public static void registerScreens(RegisterMenuScreensEvent event) {
        event.register(ModMenuTypes.TERMINAL_MENU.get(), TerminalScreen::new);
        CustomWorldMod.LOGGER.info("Registered terminal screen");
    }
}
