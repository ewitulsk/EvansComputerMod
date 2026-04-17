package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.client.ClientPacketHandler;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.network.event.RegisterPayloadHandlersEvent;
import net.neoforged.neoforge.network.registration.PayloadRegistrar;

/**
 * Handles registration of all network packets for the mod.
 */
@EventBusSubscriber(modid = EvansComputerMod.MODID)
public class ModNetwork {
    
    @SubscribeEvent
    public static void register(RegisterPayloadHandlersEvent event) {
        PayloadRegistrar registrar = event.registrar(EvansComputerMod.MODID)
                .versioned("2.0.0");
        
        // Register terminal input packet (client -> server)
        registrar.playToServer(
                TerminalInputPacket.TYPE,
                TerminalInputPacket.STREAM_CODEC,
                TerminalInputPacket::handle
        );

        // Register mouse input packet (client -> server)
        registrar.playToServer(
                MouseInputPacket.TYPE,
                MouseInputPacket.STREAM_CODEC,
                MouseInputPacket::handle
        );
        
        // Register terminal output packet (server -> client)
        // Use lambdas (not method references) to defer ClientPacketHandler class loading to client only
        registrar.playToClient(
                TerminalOutputPacket.TYPE,
                TerminalOutputPacket.STREAM_CODEC,
                (packet, ctx) -> ClientPacketHandler.handleTerminalOutput(packet, ctx)
        );

        // Register open visual editor packet (server -> client)
        registrar.playToClient(
                OpenVisualEditorPacket.TYPE,
                OpenVisualEditorPacket.STREAM_CODEC,
                (packet, ctx) -> ClientPacketHandler.handleOpenVisualEditor(packet, ctx)
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
                (packet, ctx) -> ClientPacketHandler.handleProgramListResponse(packet, ctx)
        );

        // Register load program response packet (server -> client)
        registrar.playToClient(
                LoadProgramResponsePacket.TYPE,
                LoadProgramResponsePacket.STREAM_CODEC,
                (packet, ctx) -> ClientPacketHandler.handleLoadProgramResponse(packet, ctx)
        );

        // Register delta sync packets
        registrar.playToClient(
                TerminalDeltaPacket.TYPE,
                TerminalDeltaPacket.STREAM_CODEC,
                (packet, ctx) -> ClientPacketHandler.handleTerminalDelta(packet, ctx)
        );

        registrar.playToServer(
                TerminalReadyPacket.TYPE,
                TerminalReadyPacket.STREAM_CODEC,
                TerminalReadyPacket::handle
        );

        EvansComputerMod.LOGGER.info("Registered network packets");
    }
}
