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
                // 3.2.0: GUI client rendering: ui_scene, ui_step, ui_scene_ack, ui_client_hello
                //        (scene stream UiSceneCodec.VERSION 1, UI_PROTOCOL.md section 17).
                .versioned("3.2.0");
        
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

        // The GUI clipboard and the player's real one (both ways).
        registrar.playToServer(
                ClipboardPackets.Push.TYPE,
                ClipboardPackets.Push.STREAM_CODEC,
                ClipboardPackets.Push::handle);
        // A full-screen program holds a computer's keyboard: the Terminal screen gives it Esc.
        registrar.playToClient(
                TerminalKeysPacket.TYPE,
                TerminalKeysPacket.STREAM_CODEC,
                TerminalKeysPacket::handle);
        registrar.playToClient(
                ClipboardPackets.Set.TYPE,
                ClipboardPackets.Set.STREAM_CODEC,
                ClipboardPackets.Set::handle);

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

        // GUI client rendering: scene stream (server -> client), its acks and the client hello.
        registrar.playToClient(
                UiScenePackets.Scene.TYPE,
                UiScenePackets.Scene.STREAM_CODEC,
                (packet, ctx) -> com.example.evanscomputermod.client.ClientUiSceneHooks.handleScene(packet, ctx)
        );
        registrar.playToClient(
                UiScenePackets.Step.TYPE,
                UiScenePackets.Step.STREAM_CODEC,
                (packet, ctx) -> com.example.evanscomputermod.client.ClientUiSceneHooks.handleStep(packet, ctx)
        );
        registrar.playToServer(
                UiScenePackets.Ack.TYPE,
                UiScenePackets.Ack.STREAM_CODEC,
                UiScenePackets.Ack::handle
        );
        registrar.playToServer(
                UiScenePackets.Hello.TYPE,
                UiScenePackets.Hello.STREAM_CODEC,
                UiScenePackets.Hello::handle
        );

        EvansComputerMod.LOGGER.info("Registered network packets");
    }
}
