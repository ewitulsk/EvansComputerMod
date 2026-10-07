package com.example.evanscomputermod.client;

import net.minecraft.client.gui.Font;

/**
 * The few GUI drawing calls our screens and HUD layers use, over
 * {@code GuiGraphicsExtractor} (26.1) and {@code GuiGraphics} (1.21.1).
 * Colours are ARGB; pass an opaque alpha.
 */
public final class GuiGfx {
    //? if >=26.1 {
    private final net.minecraft.client.gui.GuiGraphicsExtractor g;

    public GuiGfx(net.minecraft.client.gui.GuiGraphicsExtractor g) {
        this.g = g;
    }

    public void fill(int x0, int y0, int x1, int y1, int argb) {
        g.fill(x0, y0, x1, y1, argb);
    }

    public void text(Font font, String s, int x, int y, int argb) {
        g.text(font, s, x, y, argb);
    }
    //?} else {
    /*private final net.minecraft.client.gui.GuiGraphics g;

    public GuiGfx(net.minecraft.client.gui.GuiGraphics g) {
        this.g = g;
    }

    public void fill(int x0, int y0, int x1, int y1, int argb) {
        g.fill(x0, y0, x1, y1, argb);
    }

    public void text(Font font, String s, int x, int y, int argb) {
        g.drawString(font, s, x, y, argb);
    }*/
    //?}

    public void centeredText(Font font, String s, int cx, int y, int argb) {
        text(font, s, cx - font.width(s) / 2, y, argb);
    }

    /** A filled box with a 1-pixel border. */
    public void box(int x0, int y0, int x1, int y1, int fillArgb, int borderArgb) {
        fill(x0, y0, x1, y1, borderArgb);
        fill(x0 + 1, y0 + 1, x1 - 1, y1 - 1, fillArgb);
    }
}
