package com.example.evanscomputermod.upload;

import net.neoforged.api.distmarker.Dist;
import net.neoforged.fml.common.Mod;
import net.neoforged.neoforge.client.event.ClientPlayerNetworkEvent;
import net.neoforged.neoforge.client.event.ClientTickEvent;
import net.neoforged.neoforge.common.NeoForge;

/** The game side of the sidecar: sends picked files on the client tick ({@link ClientUploads}). */
@Mod(value = UploadSidecarMod.MODID, dist = Dist.CLIENT)
public class UploadClientMod {
    public UploadClientMod() {
        NeoForge.EVENT_BUS.addListener((ClientTickEvent.Post e) -> ClientUploads.tick());
        NeoForge.EVENT_BUS.addListener((ClientPlayerNetworkEvent.LoggingOut e) -> ClientUploads.reset());
    }
}
