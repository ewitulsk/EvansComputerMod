package com.example.evanscomputermod.computer.overlay;

//? if <=1.21.1 {

import com.example.evanscomputermod.EvansComputerMod;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.loading.FMLEnvironment;
import net.neoforged.neoforge.network.event.RegisterPayloadHandlersEvent;

/** Registers the item overlay packet and, on the client, its renderers. */
public final class OverlayNetwork {

    private OverlayNetwork() {
    }

    public static void register(IEventBus modBus) {
        modBus.addListener((RegisterPayloadHandlersEvent e) -> e.registrar(EvansComputerMod.MODID)
                .versioned("1")
                .playToClient(ItemOverlayPacket.TYPE, ItemOverlayPacket.STREAM_CODEC,
                        (packet, ctx) -> com.example.evanscomputermod.computer.overlay.client.ClientItemOverlays.handle(packet, ctx)));
        if (FMLEnvironment.dist.isClient()) {
            com.example.evanscomputermod.computer.overlay.client.ClientItemOverlays.register();
        }
    }
}
//?}
