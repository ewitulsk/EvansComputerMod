package com.example.customworld.network;

import com.example.customworld.CustomWorldMod;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.network.event.RegisterPayloadHandlersEvent;
import net.neoforged.neoforge.network.handling.DirectionalPayloadHandler;
import net.neoforged.neoforge.network.registration.PayloadRegistrar;

/**
 * Handles registration of all network packets for the mod.
 */
@EventBusSubscriber(modid = CustomWorldMod.MODID, bus = EventBusSubscriber.Bus.MOD)
public class ModNetwork {
    
    @SubscribeEvent
    public static void register(RegisterPayloadHandlersEvent event) {
        PayloadRegistrar registrar = event.registrar(CustomWorldMod.MODID)
                .versioned("1.0.0");
        
        // Register terminal input packet (client -> server)
        registrar.playToServer(
                TerminalInputPacket.TYPE,
                TerminalInputPacket.STREAM_CODEC,
                TerminalInputPacket::handle
        );
        
        // Register terminal output packet (server -> client)
        registrar.playToClient(
                TerminalOutputPacket.TYPE,
                TerminalOutputPacket.STREAM_CODEC,
                TerminalOutputPacket::handle
        );
        
        CustomWorldMod.LOGGER.info("Registered network packets");
    }
}
