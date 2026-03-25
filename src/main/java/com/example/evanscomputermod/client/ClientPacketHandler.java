package com.example.evanscomputermod.client;

import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.network.FramebufferFullPacket;
import com.example.evanscomputermod.network.FramebufferUpdatePacket;
import com.example.evanscomputermod.network.LoadProgramResponsePacket;
import com.example.evanscomputermod.network.OpenVisualEditorPacket;
import com.example.evanscomputermod.network.ProgramListResponsePacket;
import com.example.evanscomputermod.network.TerminalOutputPacket;
import net.minecraft.client.Minecraft;
import net.minecraft.world.level.Level;
import net.neoforged.neoforge.network.handling.IPayloadContext;

import java.util.Arrays;
import java.util.List;

/**
 * Client-side packet handlers. This class is only loaded on the client,
 * keeping client-only imports (Minecraft, ClientLevel, etc.) off the server classpath.
 */
public final class ClientPacketHandler {

    public static void handleTerminalOutput(TerminalOutputPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            Minecraft mc = Minecraft.getInstance();
            Level level = mc.level;

            if (level != null && level.getBlockEntity(packet.pos()) instanceof TerminalBlockEntity te) {
                te.setBufferFromString(packet.bufferContent());
                te.setCursor(packet.cursorX(), packet.cursorY());
            }
        });
    }

    public static void handleOpenVisualEditor(OpenVisualEditorPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            Minecraft mc = Minecraft.getInstance();
            mc.setScreen(new VisualProgrammingScreen(packet.pos()));
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

    public static void handleFramebufferUpdate(FramebufferUpdatePacket packet, IPayloadContext context) {
        com.example.evanscomputermod.EvansComputerMod.LOGGER.info(
                "[CLIENT] Received FramebufferUpdatePacket: pos={}, {}x{}, tiles={}",
                packet.pos(), packet.fullWidth(), packet.fullHeight(), packet.tiles().size());
        context.enqueueWork(() -> {
            ClientDisplayManager.handleUpdatePacket(packet);
        });
    }

    public static void handleFramebufferFull(FramebufferFullPacket packet, IPayloadContext context) {
        com.example.evanscomputermod.EvansComputerMod.LOGGER.info(
                "[CLIENT] Received FramebufferFullPacket: pos={}, {}x{}",
                packet.pos(), packet.width(), packet.height());
        context.enqueueWork(() -> {
            ClientDisplayManager.handleFullPacket(packet);
        });
    }
}
