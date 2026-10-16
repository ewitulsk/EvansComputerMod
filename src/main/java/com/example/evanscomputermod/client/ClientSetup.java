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
@EventBusSubscriber(modid = EvansComputerMod.MODID, value = Dist.CLIENT)
public class ClientSetup {

    @SubscribeEvent
    public static void registerScreens(RegisterMenuScreensEvent event) {
        event.register(ModMenuTypes.TERMINAL_MENU.get(), TerminalScreen::new);
        EvansComputerMod.LOGGER.info("Registered terminal screen");
    }

    @SubscribeEvent
    public static void registerBlockEntityRenderers(EntityRenderersEvent.RegisterRenderers event) {
        event.registerBlockEntityRenderer(
                ModBlockEntities.SCREEN_BLOCK_ENTITY.get(),
                ScreenBlockEntityRenderer::new);
        EvansComputerMod.LOGGER.info("Registered screen block entity renderer");
    }

    /** The Terminal item shows the panel and modules it will be placed with. */
    //? if >=26.1 {
    @SubscribeEvent
    public static void registerItemModelProperties(net.neoforged.neoforge.client.event.RegisterSelectItemModelPropertyEvent event) {
        event.register(TerminalItemState.ID, TerminalItemState.TYPE);
    }
    //?} else {
    /*@SubscribeEvent
    public static void modifyBakingResult(net.neoforged.neoforge.client.event.ModelEvent.ModifyBakingResult event) {
        TerminalItemModel.onModifyBakingResult(event);
    }*/
    //?}

    /** Displays seen in one world mean nothing in the next. */
    @SubscribeEvent
    public static void onLoggingOut(net.neoforged.neoforge.client.event.ClientPlayerNetworkEvent.LoggingOut event) {
        com.example.evanscomputermod.network.ClientDisplayCache.clear();
    }
}
