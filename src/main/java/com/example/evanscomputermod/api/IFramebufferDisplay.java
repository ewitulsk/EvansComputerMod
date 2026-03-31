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
}
