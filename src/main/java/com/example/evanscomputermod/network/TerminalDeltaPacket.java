package com.example.evanscomputermod.network;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.computer.FramebufferDiffTracker;
import com.example.evanscomputermod.computer.TerminalDisplay;
import io.netty.buffer.ByteBuf;
import net.minecraft.core.UUIDUtil;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.Identifier;

import java.io.ByteArrayOutputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.ArrayList;
import java.util.List;
import java.util.UUID;
import java.util.zip.Deflater;
import java.util.zip.Inflater;

/**
 * Delta or keyframe packet for display synchronization, addressed by
 * computer id and display id
 * ({@link com.example.evanscomputermod.api.IComputerHost#DISPLAY_OWN},
 * {@link com.example.evanscomputermod.api.IComputerHost#DISPLAY_ATTACHED}).
 * One packet may be sent to several players (see
 * {@link com.example.evanscomputermod.computer.DisplaySyncHub}), so it is
 * never mutated after construction.
 *
 * <p>Wire: computer UUID, display id, a flags byte, then for a timed packet
 * the presentation time and the server's send time (display-clock µs, var
 * longs), then the payload. Flags (DISPLAY_PIPELINE_SPEC.md Part C5):
 * <ul>
 *   <li>{@link #FLAG_TIMED}: show at {@code ptsUs} (the client's playout
 *       buffer holds it); untimed packets are applied when they arrive.</li>
 *   <li>{@link #FLAG_GFX_ONLY} / {@link #FLAG_TEXT_ONLY}: the payload has only
 *       the graphics plane / only the text plane (a timed frame never
 *       carries text, so text is never late).</li>
 *   <li>{@link #ENC_JPEG} in the encoding bits: the graphics plane is a whole
 *       JPEG frame (Part D), not exact tiles.</li>
 *   <li>{@link #FLAG_FLUSH}: a flush marker: drop queued timed frames after
 *       generation {@code n} (the payload).</li>
 * </ul>
 * Exact payloads are zlib-compressed; JPEG and flush payloads are stored.
 * Untimed exact packets have flags 0 and today's payload.
 */
public record TerminalDeltaPacket(
        UUID computerId,
        byte displayId,
        int flags,
        long ptsUs,
        long sendUs,
        byte[] compressedPayload
) implements CustomPacketPayload {

    public static final int PACKET_TYPE_DELTA = 0;
    public static final int PACKET_TYPE_KEYFRAME = 1;
    /** Parsed type of a flush marker. */
    public static final int PACKET_TYPE_FLUSH = 2;

    public static final int FLAG_TIMED = 1;
    public static final int FLAG_GFX_ONLY = 2;
    public static final int FLAG_TEXT_ONLY = 4;
    public static final int FLAG_FLUSH = 8;
    /** Encoding bits (4-5): exact tiles or a JPEG frame. */
    public static final int ENC_MASK = 0x30, ENC_EXACT = 0, ENC_JPEG = 0x10;

    public static final Type<TerminalDeltaPacket> TYPE =
            new Type<>(Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "terminal_delta"));

    /** Largest payload accepted. */
    static final int MAX_PAYLOAD = 32 << 20;

    public static final StreamCodec<ByteBuf, TerminalDeltaPacket> STREAM_CODEC = new StreamCodec<>() {
        @Override
        public TerminalDeltaPacket decode(ByteBuf buf) {
            UUID id = UUIDUtil.STREAM_CODEC.decode(buf);
            byte display = buf.readByte();
            int flags = buf.readUnsignedByte();
            long pts = 0, send = 0;
            if ((flags & FLAG_TIMED) != 0) {
                pts = readVarLong(buf);
                send = readVarLong(buf);
            }
            int n = UiScenePackets.readVar(buf);
            if (n < 0 || n > MAX_PAYLOAD) throw new IllegalArgumentException("display payload of " + n + " bytes");
            byte[] data = new byte[n];
            buf.readBytes(data);
            return new TerminalDeltaPacket(id, display, flags, pts, send, data);
        }

        @Override
        public void encode(ByteBuf buf, TerminalDeltaPacket p) {
            UUIDUtil.STREAM_CODEC.encode(buf, p.computerId());
            buf.writeByte(p.displayId());
            buf.writeByte(p.flags());
            if ((p.flags() & FLAG_TIMED) != 0) {
                writeVarLong(buf, p.ptsUs());
                writeVarLong(buf, p.sendUs());
            }
            UiScenePackets.writeVar(buf, p.compressedPayload().length);
            buf.writeBytes(p.compressedPayload());
        }
    };

    /** An untimed exact packet (flags 0), as before timed presentation. */
    public TerminalDeltaPacket(UUID computerId, byte displayId, byte[] compressedPayload) {
        this(computerId, displayId, 0, 0, 0, compressedPayload);
    }

    static void writeVarLong(ByteBuf buf, long v) {
        // zig-zag, so the rare negative time still fits
        long z = (v << 1) ^ (v >> 63);
        while ((z & ~0x7FL) != 0) {
            buf.writeByte((int) ((z & 0x7F) | 0x80));
            z >>>= 7;
        }
        buf.writeByte((int) z);
    }

    static long readVarLong(ByteBuf buf) {
        long z = 0;
        for (int shift = 0; shift < 70; shift += 7) {
            int b = buf.readByte();
            z |= (long) (b & 0x7F) << shift;
            if ((b & 0x80) == 0) return (z >>> 1) ^ -(z & 1);
        }
        throw new IllegalArgumentException("varlong too long");
    }

    @Override
    public Type<? extends CustomPacketPayload> type() {
        return TYPE;
    }

    public boolean timed() {
        return (flags & FLAG_TIMED) != 0;
    }

    public boolean jpeg() {
        return (flags & ENC_MASK) == ENC_JPEG;
    }

    public boolean flush() {
        return (flags & FLAG_FLUSH) != 0;
    }

    /** The same packet tagged to be shown at {@code pts} (sent at {@code send}). */
    public TerminalDeltaPacket withTime(long pts, long send) {
        return new TerminalDeltaPacket(computerId, displayId, flags | FLAG_TIMED, pts, send, compressedPayload);
    }

    // --- Encoding (server side) ---

    /** Create a keyframe packet with full display state. */
    public static TerminalDeltaPacket createKeyframe(UUID computerId, byte displayId, TerminalDisplay display,
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
            // Pixel format (0=indexed8, 1=rgba8888) — drives client decode of
            // both the palette and the pixel buffer below.
            raw.write(display.getPixelFormat());

            // Palette (768 bytes RGB from ARGB). Always written so the
            // client has it ready for any subsequent format switch back
            // to indexed.
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

            // Pixel data — already sized to w*h*bpp by TerminalDisplay.
            byte[] pixels = display.getPixelData();
            if (pixels != null) {
                writeInt(raw, pixels.length);
                raw.write(pixels, 0, pixels.length);
            } else {
                writeInt(raw, 0);
            }
        }

        return new TerminalDeltaPacket(computerId, displayId, compress(raw.toByteArray(), deflater));
    }

    /** Create a delta packet with only changed rows/tiles (text and graphics). */
    public static TerminalDeltaPacket createDelta(
            UUID computerId,
            byte displayId,
            long generation,
            int displayMode,
            FramebufferDiffTracker.TextDelta textDelta,
            FramebufferDiffTracker.GfxDelta gfxDelta,
            int textWidth,
            Deflater deflater
    ) {
        int flags = displayMode >= 1 && gfxDelta == null ? FLAG_TEXT_ONLY : 0;
        return createDelta(computerId, displayId, generation, displayMode, textDelta, gfxDelta, textWidth, flags, deflater);
    }

    /**
     * A delta with {@code flags}: {@link #FLAG_GFX_ONLY} leaves the text plane
     * out ({@code textDelta} may be null), {@link #FLAG_TEXT_ONLY} the graphics.
     */
    public static TerminalDeltaPacket createDelta(
            UUID computerId,
            byte displayId,
            long generation,
            int displayMode,
            FramebufferDiffTracker.TextDelta textDelta,
            FramebufferDiffTracker.GfxDelta gfxDelta,
            int textWidth,
            int flags,
            Deflater deflater
    ) {
        ByteArrayOutputStream raw = new ByteArrayOutputStream(1024);

        // Header
        raw.write(PACKET_TYPE_DELTA);
        writeLong(raw, generation);
        raw.write(displayMode);

        if ((flags & FLAG_GFX_ONLY) == 0) {
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
        }

        // Graphics delta (only if mode >= 1)
        if ((flags & FLAG_TEXT_ONLY) == 0 && displayMode >= 1 && gfxDelta != null) {
            // Pixel format byte at the top of the gfx section. Drives
            // tile-byte sizing on the parser.
            raw.write(gfxDelta.pixelFormat());
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

        return new TerminalDeltaPacket(computerId, displayId, flags & ~ENC_MASK, 0, 0, compress(raw.toByteArray(), deflater));
    }

    /**
     * A whole graphics plane as a JPEG frame (Part D): generation, mode, size,
     * the display's pixel format (the client converts back to it) and the
     * JPEG. Graphics only; stored, not deflated.
     */
    public static TerminalDeltaPacket createJpeg(UUID computerId, byte displayId, long generation, int displayMode,
                                                 int w, int h, int pixelFormat, byte[] jpeg) {
        ByteArrayOutputStream raw = new ByteArrayOutputStream(jpeg.length + 16);
        raw.write(PACKET_TYPE_DELTA);
        writeLong(raw, generation);
        raw.write(displayMode);
        writeShort(raw, w);
        writeShort(raw, h);
        raw.write(pixelFormat);
        raw.write(jpeg, 0, jpeg.length);
        return new TerminalDeltaPacket(computerId, displayId, FLAG_GFX_ONLY | ENC_JPEG, 0, 0, raw.toByteArray());
    }

    /** A flush marker of generation {@code generation}: drop queued timed frames after {@code after}. */
    public static TerminalDeltaPacket createFlush(UUID computerId, byte displayId, long generation, long after) {
        ByteArrayOutputStream raw = new ByteArrayOutputStream(17);
        writeLong(raw, generation);
        writeLong(raw, after);
        return new TerminalDeltaPacket(computerId, displayId, FLAG_FLUSH, 0, 0, raw.toByteArray());
    }

    // --- Decoding (client side) ---

    /** Parsed result from a delta packet. */
    public record ParsedDelta(
            int packetType,
            long generation,
            int displayMode,
            int pixelFormat,
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
            List<byte[]> changedTileData,
            // Timed presentation and lossy frames
            int flags,
            long ptsUs,
            long sendUs,
            /** JPEG frame: the payload as sent, and (after {@link #decodeJpeg}) the plane in the display's format. */
            byte[] jpeg,
            byte[][] decodedPlane,
            /** Flush marker: drop queued timed frames after this generation. */
            long flushAfter
    ) {
        public boolean timed() {
            return (flags & FLAG_TIMED) != 0;
        }

        public boolean hasText() {
            return (flags & FLAG_GFX_ONLY) == 0 && packetType != PACKET_TYPE_FLUSH;
        }

        public boolean hasGfx() {
            return (flags & FLAG_TEXT_ONLY) == 0 && packetType != PACKET_TYPE_FLUSH;
        }

        public boolean isJpeg() {
            return (flags & ENC_MASK) == ENC_JPEG;
        }

        /** Decode the JPEG plane (client decode thread). False if it doesn't decode. */
        public boolean decodeJpeg() {
            if (jpeg == null || decodedPlane[0] != null) return decodedPlane[0] != null;
            byte[] plane = new byte[gfxWidth * gfxHeight * TerminalDisplay.bytesPerPixel(pixelFormat)];
            if (!com.example.evanscomputermod.computer.pipeline.LossyCodec.forThisThread()
                    .decode(jpeg, gfxWidth, gfxHeight, pixelFormat, plane)) return false;
            decodedPlane[0] = plane;
            return true;
        }

        /** The decoded plane of a JPEG frame, or null. */
        public byte[] plane() {
            return decodedPlane == null ? null : decodedPlane[0];
        }
    }

    /** Decompress and parse the packet payload. */
    public ParsedDelta parse() {
        if (flush()) {
            ByteBuffer b = ByteBuffer.wrap(compressedPayload).order(ByteOrder.LITTLE_ENDIAN);
            long gen = b.getLong();
            long after = b.getLong();
            return new ParsedDelta(PACKET_TYPE_FLUSH, gen, 0, 0, null, 0, 0, 0, 0, false, null, null, 0, 0,
                    0, null, null, false, null, null, null, flags, ptsUs, sendUs, null, null, after);
        }
        if (jpeg()) {
            ByteBuffer b = ByteBuffer.wrap(compressedPayload).order(ByteOrder.LITTLE_ENDIAN);
            b.get(); // type: delta
            long gen = b.getLong();
            int mode = b.get() & 0xFF;
            int w = b.getShort() & 0xFFFF, h = b.getShort() & 0xFFFF;
            int fmt = b.get() & 0xFF;
            byte[] j = new byte[b.remaining()];
            b.get(j);
            return new ParsedDelta(PACKET_TYPE_DELTA, gen, mode, fmt, null, 0, 0, 0, 0, false, null, null, w, h,
                    0, null, null, false, null, null, null, flags, ptsUs, sendUs, j, new byte[1][], -1);
        }
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
        int pixelFormat = 0;
        byte[] palette = null;
        byte[] pixels = null;
        if (displayMode >= 1 && buf.hasRemaining()) {
            gfxW = buf.getShort() & 0xFFFF;
            gfxH = buf.getShort() & 0xFFFF;
            pixelFormat = buf.get() & 0xFF;
            palette = new byte[768];
            buf.get(palette);
            int pixLen = buf.getInt();
            if (pixLen > 0) {
                pixels = new byte[pixLen];
                buf.get(pixels);
            }
        }

        return new ParsedDelta(PACKET_TYPE_KEYFRAME, generation, displayMode, pixelFormat,
                cells, textW, textH, cursorX, cursorY, cursorVis,
                palette, pixels, gfxW, gfxH,
                0, null, null, false, null, null, null, flags, ptsUs, sendUs, null, null, -1);
    }

    private ParsedDelta parseDeltaBody(ByteBuffer buf, long generation, int displayMode) {
        int scrollOffset = 0;
        List<Integer> rowIndices = null;
        List<byte[]> rowData = null;
        int cursorX = 0, cursorY = 0;
        boolean cursorVis = false;
        if ((flags & FLAG_GFX_ONLY) == 0) {
            int textWidth = buf.getShort() & 0xFFFF;
            int rowBytes = textWidth * TerminalDisplay.CELL_SIZE;

            scrollOffset = buf.get(); // signed byte
            int numRows = buf.getShort() & 0xFFFF;
            rowIndices = new ArrayList<>(numRows);
            rowData = new ArrayList<>(numRows);

            for (int i = 0; i < numRows; i++) {
                int rowIdx = buf.getShort() & 0xFFFF;
                rowIndices.add(rowIdx);
                byte[] rd = new byte[rowBytes];
                buf.get(rd);
                rowData.add(rd);
            }

            cursorX = buf.getShort() & 0xFFFF;
            cursorY = buf.getShort() & 0xFFFF;
            cursorVis = buf.get() != 0;
        }

        int pixelFormat = -1;
        boolean paletteChanged = false;
        int[] palette = null;
        List<Integer> tileIndices = null;
        List<byte[]> tileData = null;

        if ((flags & FLAG_TEXT_ONLY) == 0 && displayMode >= 1 && buf.hasRemaining()) {
            pixelFormat = buf.get() & 0xFF;
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
            int bpp = TerminalDisplay.bytesPerPixel(pixelFormat);
            int tileBytes = 16 * 16 * bpp;
            int numTiles = buf.getShort() & 0xFFFF;
            tileIndices = new ArrayList<>(numTiles);
            tileData = new ArrayList<>(numTiles);
            for (int i = 0; i < numTiles; i++) {
                tileIndices.add(buf.getShort() & 0xFFFF);
                byte[] td = new byte[tileBytes];
                buf.get(td);
                tileData.add(td);
            }
        }

        return new ParsedDelta(PACKET_TYPE_DELTA, generation, displayMode, pixelFormat,
                null, 0, 0, cursorX, cursorY, cursorVis,
                null, null, 0, 0,
                scrollOffset, rowIndices, rowData, paletteChanged, palette,
                tileIndices, tileData, flags, ptsUs, sendUs, null, null, -1);
    }

    // --- Compression helpers ---

    private static byte[] compress(byte[] data, Deflater deflater) {
        deflater.reset();
        deflater.setInput(data);
        deflater.finish();
        ByteArrayOutputStream out = new ByteArrayOutputStream(Math.max(64, data.length / 2));
        byte[] buf = SCRATCH.get();
        while (!deflater.finished()) {
            int count = deflater.deflate(buf);
            out.write(buf, 0, count);
        }
        return out.toByteArray();
    }

    /** Per-thread deflate/inflate buffers (A7: no new scratch array per packet). */
    private static final ThreadLocal<byte[]> SCRATCH = ThreadLocal.withInitial(() -> new byte[16384]);
    private static final ThreadLocal<Inflater> INFLATER = ThreadLocal.withInitial(Inflater::new);

    private static byte[] decompress(byte[] compressed) {
        try {
            Inflater inflater = INFLATER.get();
            inflater.reset();
            inflater.setInput(compressed);
            ByteArrayOutputStream out = new ByteArrayOutputStream(compressed.length * 4);
            byte[] buf = SCRATCH.get();
            while (!inflater.finished()) {
                int count = inflater.inflate(buf);
                if (count == 0 && inflater.needsInput()) break;
                out.write(buf, 0, count);
            }
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
