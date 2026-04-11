package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.computer.FramebufferDiffTracker;
import com.example.evanscomputermod.computer.TerminalDisplay;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.BlockPos;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;

import java.io.ByteArrayOutputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.ArrayList;
import java.util.List;
import java.util.zip.Deflater;
import java.util.zip.Inflater;

/**
 * Delta or keyframe packet for terminal display synchronization.
 * Payload is zlib-compressed for bandwidth efficiency.
 */
public record TerminalDeltaPacket(
        BlockPos pos,
        byte[] compressedPayload
) implements CustomPacketPayload {

    public static final int PACKET_TYPE_DELTA = 0;
    public static final int PACKET_TYPE_KEYFRAME = 1;

    public static final Type<TerminalDeltaPacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "terminal_delta"));

    public static final StreamCodec<ByteBuf, TerminalDeltaPacket> STREAM_CODEC = StreamCodec.composite(
            BlockPos.STREAM_CODEC, TerminalDeltaPacket::pos,
            ByteBufCodecs.BYTE_ARRAY, TerminalDeltaPacket::compressedPayload,
            TerminalDeltaPacket::new
    );

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    // --- Encoding (server side) ---

    /** Create a keyframe packet with full display state. */
    public static TerminalDeltaPacket createKeyframe(BlockPos pos, TerminalDisplay display,
                                                      long generation, Deflater deflater) {
        ByteArrayOutputStream raw = new ByteArrayOutputStream(1024);

        // Header
        raw.write(PACKET_TYPE_KEYFRAME);
        writeLong(raw, generation);
        raw.write(display.getDisplayMode());

        // Text framebuffer
        int textW = display.getWidth();
        int textH = display.getHeight();
        writeShort(raw, textW);
        writeShort(raw, textH);
        writeShort(raw, display.getCursorX());
        writeShort(raw, display.getCursorY());
        raw.write(display.isCursorVisible() ? 1 : 0);
        byte[] cells = display.getCellData();
        writeInt(raw, cells.length);
        raw.write(cells, 0, cells.length);

        // Graphics framebuffer
        if (display.getDisplayMode() >= 1) {
            int gfxW = display.getGfxWidth();
            int gfxH = display.getGfxHeight();
            writeShort(raw, gfxW);
            writeShort(raw, gfxH);

            // Palette (768 bytes RGB from ARGB)
            int[] palette = display.getPalette();
            if (palette != null) {
                for (int i = 0; i < 256; i++) {
                    int argb = palette[i];
                    raw.write((argb >> 16) & 0xFF); // R
                    raw.write((argb >> 8) & 0xFF);  // G
                    raw.write(argb & 0xFF);          // B
                }
            } else {
                for (int i = 0; i < 768; i++) raw.write(0);
            }

            // Pixel data
            byte[] pixels = display.getPixelData();
            if (pixels != null) {
                writeInt(raw, pixels.length);
                raw.write(pixels, 0, pixels.length);
            } else {
                writeInt(raw, 0);
            }
        }

        return new TerminalDeltaPacket(pos, compress(raw.toByteArray(), deflater));
    }

    /** Create a delta packet with only changed rows/tiles. */
    public static TerminalDeltaPacket createDelta(
            BlockPos pos,
            long generation,
            int displayMode,
            FramebufferDiffTracker.TextDelta textDelta,
            FramebufferDiffTracker.GfxDelta gfxDelta,
            int textWidth,
            Deflater deflater
    ) {
        ByteArrayOutputStream raw = new ByteArrayOutputStream(1024);

        // Header
        raw.write(PACKET_TYPE_DELTA);
        writeLong(raw, generation);
        raw.write(displayMode);

        // Text delta — include width so client can parse row data
        writeShort(raw, textWidth);
        raw.write(textDelta.scrollOffset()); // signed byte
        writeShort(raw, textDelta.changedRowIndices().size());
        int rowBytes = textWidth * TerminalDisplay.CELL_SIZE;
        for (int i = 0; i < textDelta.changedRowIndices().size(); i++) {
            writeShort(raw, textDelta.changedRowIndices().get(i));
            byte[] rowData = textDelta.changedRowData().get(i);
            raw.write(rowData, 0, Math.min(rowData.length, rowBytes));
        }

        // Cursor
        writeShort(raw, textDelta.cursorX());
        writeShort(raw, textDelta.cursorY());
        raw.write(textDelta.cursorVisible() ? 1 : 0);

        // Graphics delta (only if mode >= 1)
        if (displayMode >= 1 && gfxDelta != null) {
            raw.write(gfxDelta.paletteChanged() ? 1 : 0);
            if (gfxDelta.paletteChanged() && gfxDelta.palette() != null) {
                for (int i = 0; i < 256; i++) {
                    int argb = gfxDelta.palette()[i];
                    raw.write((argb >> 16) & 0xFF);
                    raw.write((argb >> 8) & 0xFF);
                    raw.write(argb & 0xFF);
                }
            }
            writeShort(raw, gfxDelta.changedTileIndices().size());
            for (int i = 0; i < gfxDelta.changedTileIndices().size(); i++) {
                writeShort(raw, gfxDelta.changedTileIndices().get(i));
                byte[] tileData = gfxDelta.changedTileData().get(i);
                raw.write(tileData, 0, tileData.length);
            }
        }

        return new TerminalDeltaPacket(pos, compress(raw.toByteArray(), deflater));
    }

    // --- Decoding (client side) ---

    /** Parsed result from a delta packet. */
    public record ParsedDelta(
            int packetType,
            long generation,
            int displayMode,
            // Keyframe fields
            byte[] fullTextData,     // header+cells for keyframe
            int textWidth, int textHeight,
            int cursorX, int cursorY, boolean cursorVisible,
            byte[] fullPalette,      // 768 bytes RGB
            byte[] fullPixelData,
            int gfxWidth, int gfxHeight,
            // Delta fields
            int scrollOffset,
            List<Integer> changedRowIndices,
            List<byte[]> changedRowData,
            boolean paletteChanged,
            int[] palette,           // 256 ARGB entries
            List<Integer> changedTileIndices,
            List<byte[]> changedTileData
    ) {}

    /** Decompress and parse the packet payload. */
    public ParsedDelta parse() {
        byte[] raw = decompress(compressedPayload);
        ByteBuffer buf = ByteBuffer.wrap(raw).order(ByteOrder.LITTLE_ENDIAN);

        int packetType = buf.get() & 0xFF;
        long generation = buf.getLong();
        int displayMode = buf.get() & 0xFF;

        if (packetType == PACKET_TYPE_KEYFRAME) {
            return parseKeyframe(buf, generation, displayMode);
        } else {
            return parseDeltaBody(buf, generation, displayMode);
        }
    }

    private ParsedDelta parseKeyframe(ByteBuffer buf, long generation, int displayMode) {
        int textW = buf.getShort() & 0xFFFF;
        int textH = buf.getShort() & 0xFFFF;
        int cursorX = buf.getShort() & 0xFFFF;
        int cursorY = buf.getShort() & 0xFFFF;
        boolean cursorVis = buf.get() != 0;
        int cellLen = buf.getInt();
        byte[] cells = new byte[cellLen];
        buf.get(cells);

        int gfxW = 0, gfxH = 0;
        byte[] palette = null;
        byte[] pixels = null;
        if (displayMode >= 1 && buf.hasRemaining()) {
            gfxW = buf.getShort() & 0xFFFF;
            gfxH = buf.getShort() & 0xFFFF;
            palette = new byte[768];
            buf.get(palette);
            int pixLen = buf.getInt();
            if (pixLen > 0) {
                pixels = new byte[pixLen];
                buf.get(pixels);
            }
        }

        return new ParsedDelta(PACKET_TYPE_KEYFRAME, generation, displayMode,
                cells, textW, textH, cursorX, cursorY, cursorVis,
                palette, pixels, gfxW, gfxH,
                0, null, null, false, null, null, null);
    }

    private ParsedDelta parseDeltaBody(ByteBuffer buf, long generation, int displayMode) {
        int textWidth = buf.getShort() & 0xFFFF;
        int rowBytes = textWidth * TerminalDisplay.CELL_SIZE;

        int scrollOffset = buf.get(); // signed byte
        int numRows = buf.getShort() & 0xFFFF;
        List<Integer> rowIndices = new ArrayList<>(numRows);
        List<byte[]> rowData = new ArrayList<>(numRows);

        for (int i = 0; i < numRows; i++) {
            int rowIdx = buf.getShort() & 0xFFFF;
            rowIndices.add(rowIdx);
            byte[] rd = new byte[rowBytes];
            buf.get(rd);
            rowData.add(rd);
        }

        int cursorX = buf.getShort() & 0xFFFF;
        int cursorY = buf.getShort() & 0xFFFF;
        boolean cursorVis = buf.get() != 0;

        boolean paletteChanged = false;
        int[] palette = null;
        List<Integer> tileIndices = null;
        List<byte[]> tileData = null;

        if (displayMode >= 1 && buf.hasRemaining()) {
            paletteChanged = buf.get() != 0;
            if (paletteChanged) {
                palette = new int[256];
                for (int i = 0; i < 256; i++) {
                    int r = buf.get() & 0xFF;
                    int g = buf.get() & 0xFF;
                    int b = buf.get() & 0xFF;
                    palette[i] = 0xFF000000 | (r << 16) | (g << 8) | b;
                }
            }
            int numTiles = buf.getShort() & 0xFFFF;
            tileIndices = new ArrayList<>(numTiles);
            tileData = new ArrayList<>(numTiles);
            for (int i = 0; i < numTiles; i++) {
                tileIndices.add(buf.getShort() & 0xFFFF);
                byte[] td = new byte[16 * 16];
                buf.get(td);
                tileData.add(td);
            }
        }

        return new ParsedDelta(PACKET_TYPE_DELTA, generation, displayMode,
                null, 0, 0, cursorX, cursorY, cursorVis,
                null, null, 0, 0,
                scrollOffset, rowIndices, rowData, paletteChanged, palette,
                tileIndices, tileData);
    }

    // --- Compression helpers ---

    private static byte[] compress(byte[] data, Deflater deflater) {
        deflater.reset();
        deflater.setInput(data);
        deflater.finish();
        ByteArrayOutputStream out = new ByteArrayOutputStream(data.length);
        byte[] buf = new byte[4096];
        while (!deflater.finished()) {
            int count = deflater.deflate(buf);
            out.write(buf, 0, count);
        }
        return out.toByteArray();
    }

    private static byte[] decompress(byte[] compressed) {
        try {
            Inflater inflater = new Inflater();
            inflater.setInput(compressed);
            ByteArrayOutputStream out = new ByteArrayOutputStream(compressed.length * 4);
            byte[] buf = new byte[4096];
            while (!inflater.finished()) {
                int count = inflater.inflate(buf);
                if (count == 0 && inflater.needsInput()) break;
                out.write(buf, 0, count);
            }
            inflater.end();
            return out.toByteArray();
        } catch (Exception e) {
            return new byte[0];
        }
    }

    // --- Binary write helpers ---

    private static void writeShort(ByteArrayOutputStream out, int value) {
        out.write(value & 0xFF);
        out.write((value >> 8) & 0xFF);
    }

    private static void writeInt(ByteArrayOutputStream out, int value) {
        out.write(value & 0xFF);
        out.write((value >> 8) & 0xFF);
        out.write((value >> 16) & 0xFF);
        out.write((value >> 24) & 0xFF);
    }

    private static void writeLong(ByteArrayOutputStream out, long value) {
        for (int i = 0; i < 8; i++) {
            out.write((int) ((value >> (i * 8)) & 0xFF));
        }
    }
}
