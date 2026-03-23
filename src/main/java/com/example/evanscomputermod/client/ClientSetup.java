package com.example.evanscomputermod.client;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.ModBlockEntities;
import com.example.evanscomputermod.block.ModMenuTypes;
import com.example.evanscomputermod.block.TerminalScreen;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.client.event.EntityRenderersEvent;
import net.neoforged.neoforge.client.event.RegisterMenuScreensEvent;

/**
 * Client-side setup and event handlers.
 */
@EventBusSubscriber(modid = EvansComputerMod.MODID, bus = EventBusSubscriber.Bus.MOD, value = Dist.CLIENT)
public class ClientSetup {

    @SubscribeEvent
    public static void registerScreens(RegisterMenuScreensEvent event) {
        event.register(ModMenuTypes.TERMINAL_MENU.get(), TerminalScreen::new);
        EvansComputerMod.LOGGER.info("Registered terminal screen");
    }

    @SubscribeEvent
    public static void registerRenderers(EntityRenderersEvent.RegisterRenderers event) {
        event.registerBlockEntityRenderer(ModBlockEntities.DISPLAY_BLOCK_ENTITY.get(),
                DisplayBlockEntityRenderer::new);
        EvansComputerMod.LOGGER.info("Registered display block entity renderer");
    }
}
