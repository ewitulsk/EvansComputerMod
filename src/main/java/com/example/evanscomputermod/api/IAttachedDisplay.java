package com.example.evanscomputermod.api;

import com.example.evanscomputermod.computer.TerminalDisplay;

/**
 * A second display attached to a computer, next to its own: on a Terminal,
 * the in-world Screen cluster beside it. Programs reach it as the
 * {@code screen} target and clients as display id
 * {@link IComputerHost#DISPLAY_ATTACHED}.
 *
 * <p>Methods may be called from the computer's worker and program threads,
 * so implementations read volatile state and schedule world changes on the
 * server thread themselves.
 */
public interface IAttachedDisplay {

    /** The scanout that is streamed to clients. */
    TerminalDisplay display();

    /** Native graphics width in pixels (what {@code screen_get_gfx_width} reports). */
    int gfxWidth();

    /** Native graphics height in pixels. */
    int gfxHeight();

    /** Turn the display on or off ({@code screen_set_power}). */
    void setPowered(boolean on);

    /** Panel tier the display runs at (a Screen wall: its lowest block's). Default: Tier 2. */
    default com.example.evanscomputermod.computer.display.PanelTier tier() {
        return com.example.evanscomputermod.computer.display.PanelTier.COLOR_LCD;
    }
}
