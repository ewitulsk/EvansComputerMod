package com.example.evanscomputermod.api;

/**
 * Capability for hosts that have a memory-mapped framebuffer display.
 * The framebuffer uses a cell-based format: each cell is 4 bytes
 * (character, attribute, flags, reserved).
 *
 * <p>Attribute byte: low nibble = foreground color (0-15),
 * high nibble = background color (0-15).</p>
 *
 * <p>Flags byte: bit 0 = bold, bit 1 = underline, bit 2 = blink,
 * bit 3 = inverse.</p>
 */
public interface IFramebufferDisplay {

    int getWidth();

    int getHeight();

    int getCursorX();

    int getCursorY();

    boolean isCursorVisible();

    /**
     * Raw cell data: width * height * 4 bytes.
     * Each cell is [char, attribute, flags, reserved].
     */
    byte[] getCellData();

    /**
     * Set the display from a raw framebuffer region.
     * The data starts with a 64-byte header followed by cell data.
     */
    void setFromBytes(byte[] data);

    /**
     * Get the character at position (x, y).
     */
    byte getCharAt(int x, int y);

    /**
     * Get the attribute byte at position (x, y).
     */
    byte getAttrAt(int x, int y);

    /**
     * Get the flags byte at position (x, y).
     */
    byte getFlagsAt(int x, int y);

    // --- Graphics framebuffer support ---

    /**
     * Display mode: 0=text-only, 1=graphics-only, 2=overlay (graphics + text).
     */
    default int getDisplayMode() { return 0; }

    /** Graphics framebuffer width in pixels (0 if not initialized). */
    default int getGfxWidth() { return 0; }

    /** Graphics framebuffer height in pixels (0 if not initialized). */
    default int getGfxHeight() { return 0; }

    /** 256-entry ARGB palette, or null if no graphics. */
    default int[] getPalette() { return null; }

    /** Indexed pixel data (width*height bytes), or null if no graphics. */
    default byte[] getPixelData() { return null; }
}
