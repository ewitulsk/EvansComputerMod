package com.example.evanscomputermod.client;

import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.network.ClientDisplayCache;
import com.example.evanscomputermod.network.TerminalDeltaPacket;
import com.example.evanscomputermod.network.TerminalReadyPacket;
import net.minecraft.client.Minecraft;
import net.neoforged.neoforge.network.handling.IPayloadContext;
import net.neoforged.neoforge.client.network.ClientPacketDistributor;

import java.util.Arrays;
import java.util.List;
import java.util.UUID;

/**
 * Client-side packet handlers. This class is only loaded on the client,
 * keeping client-only imports (Minecraft, ClientLevel, etc.) off the server classpath.
 */
public final class ClientPacketHandler {

    /**
     * A display frame for a computer. Applied to the client display cache by
     * computer id and display id, so no block entity has to be found first
     * (a terminal inside a Sable plot used to need a special lookup here).
     * Every packet is acked, which also paces the server.
     */
    public static void handleTerminalDelta(TerminalDeltaPacket packet, IPayloadContext context) {
        // Network thread: inflate, parse and decode on the decode thread, ack on receipt
        // (DISPLAY_PIPELINE_SPEC.md Part A5); the render thread only applies.
        long recvUs = com.example.evanscomputermod.network.ClockSync.clientNowUs();
        DisplayClientHooks.decode(() -> {
            if (Minecraft.getInstance().level == null) return;
            TerminalDeltaPacket.ParsedDelta delta = packet.parse();
            if (delta.isJpeg()) delta.decodeJpeg();
            ClientPacketDistributor.sendToServer(
                    new TerminalReadyPacket(packet.computerId(), packet.displayId(), delta.generation(), recvUs));
            // A delta for a display this client never had a keyframe for is dropped by the cache
            // (it came into view after the server's state was set up); a keyframe is on the way.
            DisplayClientHooks.toMain(() -> com.example.evanscomputermod.network.ClientPlayout.receive(
                    packet.computerId(), packet.displayId(), delta, recvUs));
        });
    }

    /** Ask the server for a keyframe of a computer's own display (generation 0 = "I have nothing"). */
    public static void requestKeyframe(UUID computerId) {
        ClientPacketDistributor.sendToServer(
                new TerminalReadyPacket(computerId, (byte) IComputerHost.DISPLAY_OWN, 0));
    }

    /** Text an app on a computer copied: the player's real clipboard. */
    public static void setClipboard(String text) {
        net.minecraft.client.Minecraft.getInstance().keyboardHandler.setClipboard(text);
    }
}
