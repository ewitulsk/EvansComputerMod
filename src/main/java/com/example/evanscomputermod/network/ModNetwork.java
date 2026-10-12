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
                // 3.0.0: display and visual-editor packets address computers by
                // UUID + display id instead of BlockPos.
                // 3.1.0: KeyInputPacket (raw keys and text for the GUI).
                .versioned("3.1.0");
        
        // Register terminal input packet (client -> server)
        registrar.playToServer(
                TerminalInputPacket.TYPE,
                TerminalInputPacket.STREAM_CODEC,
                TerminalInputPacket::handle
        );

        // Raw keys (HID usages) and typed text for the GUI (client -> server)
        registrar.playToServer(
                KeyInputPacket.TYPE,
                KeyInputPacket.STREAM_CODEC,
                KeyInputPacket::handle
        );

        // Register mouse input packet (client -> server)
        registrar.playToServer(
                MouseInputPacket.TYPE,
                MouseInputPacket.STREAM_CODEC,
                MouseInputPacket::handle
        );
        
        // Wireless controller
        registrar.playToServer(
                com.example.evanscomputermod.controller.ControllerPackets.Input.TYPE,
                com.example.evanscomputermod.controller.ControllerPackets.Input.STREAM_CODEC,
                com.example.evanscomputermod.controller.ControllerServer::handleInput);
        registrar.playToServer(
                com.example.evanscomputermod.controller.ControllerPackets.Bindings.TYPE,
                com.example.evanscomputermod.controller.ControllerPackets.Bindings.STREAM_CODEC,
                com.example.evanscomputermod.controller.ControllerServer::handleBindings);
        registrar.playToClient(
                com.example.evanscomputermod.controller.ControllerPackets.Status.TYPE,
                com.example.evanscomputermod.controller.ControllerPackets.Status.STREAM_CODEC,
                (packet, ctx) -> ctx.enqueueWork(() ->
                        com.example.evanscomputermod.controller.client.ControllerClient.onStatus(packet)));

        // Speaker audio (server -> client)
        registrar.playToClient(
                com.example.evanscomputermod.speaker.SpeakerAudioPacket.TYPE,
                com.example.evanscomputermod.speaker.SpeakerAudioPacket.STREAM_CODEC,
                (packet, ctx) -> ctx.enqueueWork(() ->
                        com.example.evanscomputermod.speaker.client.SpeakerClient.onAudio(packet)));

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
