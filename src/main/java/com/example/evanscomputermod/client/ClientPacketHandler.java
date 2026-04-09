package com.example.evanscomputermod.client;

import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.network.LoadProgramResponsePacket;
import com.example.evanscomputermod.network.OpenVisualEditorPacket;
import com.example.evanscomputermod.network.ProgramListResponsePacket;
import com.example.evanscomputermod.network.TerminalDeltaPacket;
import com.example.evanscomputermod.network.TerminalOutputPacket;
import com.example.evanscomputermod.network.TerminalReadyPacket;
import net.minecraft.client.Minecraft;
import net.minecraft.world.level.Level;
import net.neoforged.neoforge.network.handling.IPayloadContext;
import net.neoforged.neoforge.client.network.ClientPacketDistributor;

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
                te.getDisplay().setFromBytes(packet.framebufferData());
            }
        });
    }

    public static void handleTerminalDelta(TerminalDeltaPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            Minecraft mc = Minecraft.getInstance();
            Level level = mc.level;

            if (level != null && level.getBlockEntity(packet.pos()) instanceof TerminalBlockEntity te) {
                TerminalDisplay display = te.getDisplay();
                TerminalDeltaPacket.ParsedDelta delta = packet.parse();

                if (delta.packetType() == TerminalDeltaPacket.PACKET_TYPE_KEYFRAME) {
                    applyKeyframe(display, delta);
                } else {
                    applyDelta(display, delta);
                }

                // Send ready ack back to server
                ClientPacketDistributor.sendToServer(
                        new TerminalReadyPacket(packet.pos(), delta.generation()));
            }
        });
    }

    private static void applyKeyframe(TerminalDisplay display, TerminalDeltaPacket.ParsedDelta delta) {
        // Reconstruct text framebuffer bytes with proper header
        byte[] cells = delta.fullTextData();
        if (cells != null) {
            // Build header + cells for setFromBytes
            int cellBytes = cells.length;
            byte[] fullFb = new byte[TerminalDisplay.HEADER_SIZE + cellBytes];
            java.nio.ByteBuffer buf = java.nio.ByteBuffer.wrap(fullFb).order(java.nio.ByteOrder.LITTLE_ENDIAN);
            buf.putShort(0, (short) TerminalDisplay.FB_MAGIC);
            buf.putShort(2, (short) delta.textWidth());
            buf.putShort(4, (short) delta.textHeight());
            buf.putShort(6, (short) delta.cursorX());
            buf.putShort(8, (short) delta.cursorY());
            fullFb[10] = (byte) (delta.cursorVisible() ? 1 : 0);
            System.arraycopy(cells, 0, fullFb, TerminalDisplay.HEADER_SIZE, cellBytes);
            display.setFromBytes(fullFb);
        }

        // Apply graphics keyframe
        if (delta.displayMode() >= 1 && delta.fullPalette() != null) {
            int gfxW = delta.gfxWidth();
            int gfxH = delta.gfxHeight();
            // Build GFX data for setGfxFromBytes
            int pixLen = delta.fullPixelData() != null ? delta.fullPixelData().length : 0;
            int totalSize = 0x400 + pixLen;
            byte[] gfxData = new byte[totalSize];
            java.nio.ByteBuffer gbuf = java.nio.ByteBuffer.wrap(gfxData).order(java.nio.ByteOrder.LITTLE_ENDIAN);
            gbuf.putShort(0, (short) TerminalDisplay.GFX_MAGIC);
            gfxData[2] = (byte) delta.displayMode();
            gbuf.putShort(4, (short) gfxW);
            gbuf.putShort(6, (short) gfxH);
            // Copy palette
            System.arraycopy(delta.fullPalette(), 0, gfxData, 0x40, Math.min(delta.fullPalette().length, 768));
            // Copy pixels
            if (delta.fullPixelData() != null) {
                System.arraycopy(delta.fullPixelData(), 0, gfxData, 0x400, pixLen);
            }
            int prevPixDirty = display.getPixelDirtyCounter();
            int prevPalDirty = display.getPaletteDirtyCounter();
            display.setGfxFromBytes(gfxData);
            // setGfxFromBytes resets counters to 0 from packet data.
            // Force monotonic increase so GPU texture re-uploads every keyframe.
            display.setGfxDirtyCounters(prevPixDirty + 1, prevPalDirty + 1);
        }
    }

    private static void applyDelta(TerminalDisplay display, TerminalDeltaPacket.ParsedDelta delta) {
        // Update display mode so client switches between text/graphics rendering
        display.setDisplayMode(delta.displayMode());

        byte[] cells = display.getCellData();
        int width = display.getWidth();
        int rowBytes = width * TerminalDisplay.CELL_SIZE;

        // Apply scroll
        if (delta.scrollOffset() > 0) {
            // Scroll up: shift rows up by offset
            int offset = delta.scrollOffset();
            System.arraycopy(cells, offset * rowBytes, cells, 0, (display.getHeight() - offset) * rowBytes);
        } else if (delta.scrollOffset() < 0) {
            // Scroll down: shift rows down
            int offset = -delta.scrollOffset();
            System.arraycopy(cells, 0, cells, offset * rowBytes, (display.getHeight() - offset) * rowBytes);
        }

        // Apply changed rows
        if (delta.changedRowIndices() != null) {
            for (int i = 0; i < delta.changedRowIndices().size(); i++) {
                int rowIdx = delta.changedRowIndices().get(i);
                byte[] rowData = delta.changedRowData().get(i);
                if (rowIdx >= 0 && rowIdx < display.getHeight() && rowData != null) {
                    System.arraycopy(rowData, 0, cells, rowIdx * rowBytes,
                            Math.min(rowData.length, rowBytes));
                }
            }
        }

        // Apply cursor position from delta
        display.setCursorX(delta.cursorX());
        display.setCursorY(delta.cursorY());
        display.setCursorVisible(delta.cursorVisible());

        // Apply palette changes
        if (delta.paletteChanged() && delta.palette() != null) {
            int[] displayPalette = display.getPalette();
            if (displayPalette != null) {
                System.arraycopy(delta.palette(), 0, displayPalette, 0,
                        Math.min(delta.palette().length, displayPalette.length));
            }
        }

        // Apply tile changes to pixel data
        boolean gfxUpdated = false;
        if (delta.changedTileIndices() != null && display.getPixelData() != null) {
            byte[] pixelData = display.getPixelData();
            int gfxW = display.getGfxWidth();
            int tilesPerRow = (gfxW + 15) / 16;

            for (int i = 0; i < delta.changedTileIndices().size(); i++) {
                int tileIdx = delta.changedTileIndices().get(i);
                byte[] tileData = delta.changedTileData().get(i);
                int tileX = (tileIdx % tilesPerRow) * 16;
                int tileY = (tileIdx / tilesPerRow) * 16;

                for (int py = 0; py < 16 && tileY + py < display.getGfxHeight(); py++) {
                    for (int px = 0; px < 16 && tileX + px < gfxW; px++) {
                        pixelData[(tileY + py) * gfxW + tileX + px] = tileData[py * 16 + px];
                    }
                }
            }
            gfxUpdated = true;
        }

        // Bump dirty counters so TerminalScreen re-uploads the graphics texture to GPU
        if (gfxUpdated || delta.paletteChanged()) {
            display.setGfxDirtyCounters(
                    display.getPixelDirtyCounter() + 1,
                    display.getPaletteDirtyCounter() + 1);
        }
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
}
