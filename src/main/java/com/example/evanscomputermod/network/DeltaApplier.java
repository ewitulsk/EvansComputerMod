package com.example.evanscomputermod.network;

import com.example.evanscomputermod.computer.TerminalDisplay;

import java.util.Arrays;

/**
 * Applies a {@link TerminalDeltaPacket} to a client-side {@link TerminalDisplay}.
 * Lives outside the client package (it only touches the display) so tests can
 * run exactly the code a client runs, against a server-side tracker.
 */
public final class DeltaApplier {
    public static void applyKeyframe(TerminalDisplay display, TerminalDeltaPacket.ParsedDelta delta) {
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
            gfxData[TerminalDisplay.GFX_OFF_PIXEL_FORMAT] = (byte) delta.pixelFormat();
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

    public static void applyDelta(TerminalDisplay display, TerminalDeltaPacket.ParsedDelta delta) {
        // Update display mode + pixel format so client switches between
        // text/graphics rendering and indexed/rgba tile decoding.
        display.setDisplayMode(delta.displayMode());
        display.setPixelFormat(delta.pixelFormat());

        byte[] cells = display.getCellData();
        int width = display.getWidth();
        int rowBytes = width * TerminalDisplay.CELL_SIZE;

        // Apply scroll. After the shift, the revealed rows must be blanked to
        // match the server VTE's scroll_up / scroll_down behaviour — otherwise
        // rows the tracker treats as "unchanged blank" on the server leave
        // stale content visible on the client.
        if (delta.scrollOffset() > 0) {
            int offset = delta.scrollOffset();
            int shiftBytes = (display.getHeight() - offset) * rowBytes;
            System.arraycopy(cells, offset * rowBytes, cells, 0, shiftBytes);
            Arrays.fill(cells, shiftBytes, shiftBytes + offset * rowBytes, (byte) 0);
        } else if (delta.scrollOffset() < 0) {
            int offset = -delta.scrollOffset();
            System.arraycopy(cells, 0, cells, offset * rowBytes, (display.getHeight() - offset) * rowBytes);
            Arrays.fill(cells, 0, offset * rowBytes, (byte) 0);
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
            int gfxH = display.getGfxHeight();
            int bpp = display.getBytesPerPixel();
            int tilesPerRow = (gfxW + 15) / 16;
            int rowStrideBytes = gfxW * bpp;
            int tileRowBytes = 16 * bpp;

            for (int i = 0; i < delta.changedTileIndices().size(); i++) {
                int tileIdx = delta.changedTileIndices().get(i);
                byte[] tileData = delta.changedTileData().get(i);
                int tileX = (tileIdx % tilesPerRow) * 16;
                int tileY = (tileIdx / tilesPerRow) * 16;

                int copyW = Math.min(16, gfxW - tileX);
                if (copyW <= 0) continue;
                int copyH = Math.min(16, gfxH - tileY);
                int copyBytes = copyW * bpp;
                for (int py = 0; py < copyH; py++) {
                    int dstOff = (tileY + py) * rowStrideBytes + tileX * bpp;
                    int srcOff = py * tileRowBytes;
                    System.arraycopy(tileData, srcOff, pixelData, dstOff, copyBytes);
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

    private DeltaApplier() {}
}
