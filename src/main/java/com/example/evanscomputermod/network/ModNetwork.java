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

        // Register save visual program packet (client -> server)
        registrar.playToServer(
                SaveVisualProgramPacket.TYPE,
                SaveVisualProgramPacket.STREAM_CODEC,
                SaveVisualProgramPacket::handle
        );

        // Register request program list packet (client -> server)
        registrar.playToServer(
                RequestProgramListPacket.TYPE,
                RequestProgramListPacket.STREAM_CODEC,
                RequestProgramListPacket::handle
        );

        // Register load visual program packet (client -> server)
        registrar.playToServer(
                LoadVisualProgramPacket.TYPE,
                LoadVisualProgramPacket.STREAM_CODEC,
                LoadVisualProgramPacket::handle
        );

        // Register program list response packet (server -> client)
        registrar.playToClient(
                ProgramListResponsePacket.TYPE,
                ProgramListResponsePacket.STREAM_CODEC,
                ProgramListResponsePacket::handle
        );

        // Register load program response packet (server -> client)
        registrar.playToClient(
                LoadProgramResponsePacket.TYPE,
                LoadProgramResponsePacket.STREAM_CODEC,
                LoadProgramResponsePacket::handle
        );

        // Register framebuffer update packet (server -> client)
        registrar.playToClient(
                FramebufferUpdatePacket.TYPE,
                FramebufferUpdatePacket.STREAM_CODEC,
                FramebufferUpdatePacket::handle
        );

        // Register framebuffer full packet (server -> client)
        registrar.playToClient(
                FramebufferFullPacket.TYPE,
                FramebufferFullPacket.STREAM_CODEC,
                FramebufferFullPacket::handle
        );

        EvansComputerMod.LOGGER.info("Registered network packets");
    }
}
