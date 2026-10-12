package com.example.evanscomputermod.client;

import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.network.ClientDisplayCache;
import com.example.evanscomputermod.network.LoadProgramResponsePacket;
import com.example.evanscomputermod.network.OpenVisualEditorPacket;
import com.example.evanscomputermod.network.ProgramListResponsePacket;
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
        context.enqueueWork(() -> {
            if (Minecraft.getInstance().level == null) return;
            TerminalDeltaPacket.ParsedDelta delta = packet.parse();
            ClientDisplayCache.apply(packet.computerId(), packet.displayId(), delta);
            // else: a delta for a display this client never had a keyframe for (it
            // came into view after the server's state was set up); a keyframe is on the way.
            ClientPacketDistributor.sendToServer(
                    new TerminalReadyPacket(packet.computerId(), packet.displayId(), delta.generation()));
        });
    }

    /** Ask the server for a keyframe of a computer's own display (generation 0 = "I have nothing"). */
    public static void requestKeyframe(UUID computerId) {
        ClientPacketDistributor.sendToServer(
                new TerminalReadyPacket(computerId, (byte) IComputerHost.DISPLAY_OWN, 0));
    }

    public static void handleOpenVisualEditor(OpenVisualEditorPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            Minecraft mc = Minecraft.getInstance();
            mc.setScreen(new VisualProgrammingScreen(packet.computerId()));
        });
    }

    public static void handleProgramListResponse(ProgramListResponsePacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            Minecraft mc = Minecraft.getInstance();
            if (mc.screen instanceof VisualProgrammingScreen vps) {
                List<String> programs = packet.fileListNewlineSeparated().isEmpty()
                        ? List.of()
                        : Arrays.asList(packet.fileListNewlineSeparated().split("\n"));
                vps.onProgramListReceived(programs);
            }
        });
    }

    public static void handleLoadProgramResponse(LoadProgramResponsePacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            Minecraft mc = Minecraft.getInstance();
            if (mc.screen instanceof VisualProgrammingScreen vps) {
                vps.onProgramLoaded(packet.jsonContent());
            }
        });
    }
}
