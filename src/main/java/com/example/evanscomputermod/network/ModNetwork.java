package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.network.event.RegisterPayloadHandlersEvent;
import net.neoforged.neoforge.network.handling.DirectionalPayloadHandler;
import net.neoforged.neoforge.network.registration.PayloadRegistrar;

/**
 * Handles registration of all network packets for the mod.
 */
@EventBusSubscriber(modid = EvansComputerMod.MODID, bus = EventBusSubscriber.Bus.MOD)
public class ModNetwork {
    
    @SubscribeEvent
    public static void register(RegisterPayloadHandlersEvent event) {
        PayloadRegistrar registrar = event.registrar(EvansComputerMod.MODID)
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

        // Register open visual editor packet (server -> client)
        registrar.playToClient(
                OpenVisualEditorPacket.TYPE,
                OpenVisualEditorPacket.STREAM_CODEC,
                OpenVisualEditorPacket::handle
        );

        // Register run visual script packet (client -> server)
        registrar.playToServer(
                RunVisualScriptPacket.TYPE,
                RunVisualScriptPacket.STREAM_CODEC,
                RunVisualScriptPacket::handle
        );

        EvansComputerMod.LOGGER.info("Registered network packets");
    }
}
