package com.example.evanscomputermod.storage.client;

//? if <=1.21.1 {

import com.example.evanscomputermod.storage.StorageContent;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.client.event.EntityRenderersEvent;
import net.neoforged.neoforge.client.event.RegisterMenuScreensEvent;
import net.neoforged.neoforge.common.NeoForge;

/** Client setup for item storage. */
public final class StorageClient {

    private StorageClient() {
    }

    public static void register(IEventBus modBus) {
        modBus.addListener((RegisterMenuScreensEvent e) -> e.register(StorageContent.DRIVE_MENU.get(), DriveScreen::new));
        modBus.addListener((EntityRenderersEvent.RegisterRenderers e) ->
                e.registerBlockEntityRenderer(StorageContent.DRIVE_BE.get(), DriveRenderer::new));
        // Development only: runs when the game directory holds an ecm-storage-client-check file.
        NeoForge.EVENT_BUS.addListener(StorageClientCheck::onScreen);
        NeoForge.EVENT_BUS.addListener(StorageClientCheck::tick);
    }
}
//?}
