package com.example.evanscomputermod.api;

import com.example.evanscomputermod.computer.Framebuffer;

import javax.annotation.Nullable;

/**
 * Capability for hosts that have a pixel display (framebuffer).
 * Implemented by DisplayBlockEntity. The Terminal block does NOT implement this —
 * it discovers an adjacent DisplayBlockEntity and uses it.
 */
public interface IFramebufferHost {

    /** Returns the framebuffer for this display, or null if not initialized. */
    @Nullable
    Framebuffer getFramebuffer();

    /** Display width in pixels. */
    int getDisplayWidth();

    /** Display height in pixels. */
    int getDisplayHeight();
}
