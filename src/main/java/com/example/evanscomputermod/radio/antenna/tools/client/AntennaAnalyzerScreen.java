package com.example.evanscomputermod.radio.antenna.tools.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.antenna.tools.AnalyzerPackets;
import com.example.evanscomputermod.radio.antenna.tools.SwrPlot;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.network.chat.Component;

import java.util.List;
import java.util.Locale;

/**
 * The antenna analyzer's screen: SWR against frequency across the antenna's
 * band (log SWR axis 1:1..10:1, the 2:1 line highlighted, the 2:1 band
 * shaded, the best point marked), with the summary and the power-limit line
 * underneath. Opened by sneak + right-click on a feed point; the server sends
 * the data ({@link AnalyzerPackets.Plot}) and a refresh when a pending solve
 * lands.
 */
public class AntennaAnalyzerScreen extends Screen {
    private static final int PANEL_W = 320, PANEL_H = 200;
    private static final int BG = 0xF0101418, FRAME = 0xFF5A6470, GRID = 0xFF2C343C, LINE_2TO1 = 0xFFC8A000,
            BAND = 0x3040C040, CURVE = 0xFF40E060, MARK = 0xFFFF5050, TEXT = 0xFFE0E0E0, DIM = 0xFF9AA4AE;

    private AnalyzerPackets.Plot plot;

    AntennaAnalyzerScreen(AnalyzerPackets.Plot plot) {
        super(Component.translatable("screen.evanscomputermod.antenna_analyzer"));
        this.plot = plot;
    }

    /** Opens the screen, or refreshes it if it is already showing this feed point. */
    public static void show(AnalyzerPackets.Plot p) {
        Minecraft mc = Minecraft.getInstance();
        if (mc.screen instanceof AntennaAnalyzerScreen s) {
            if (s.plot.pos().equals(p.pos())) {
                s.plot = p;
                return;
            }
        }
        mc.setScreen(new AntennaAnalyzerScreen(p));
    }

    @Override
    public boolean isPauseScreen() {
        return false;
    }

    @Override
    public void render(GuiGraphics g, int mouseX, int mouseY, float partial) {
        super.render(g, mouseX, mouseY, partial);
        int x0 = (width - PANEL_W) / 2, y0 = (height - PANEL_H) / 2;
        g.fill(x0, y0, x0 + PANEL_W, y0 + PANEL_H, BG);
        g.renderOutline(x0, y0, PANEL_W, PANEL_H, FRAME);
        g.drawString(font, title.getString() + (plot.pending() ? " (solving...)" : ""), x0 + 6, y0 + 5, TEXT);

        int left = x0 + 34, top = y0 + 20, w = PANEL_W - 44, h = PANEL_H - 76;
        SwrPlot sp = new SwrPlot(plot.hz(), plot.swr());
        if (plot.hz().length < 2) {
            g.drawWordWrap(font, Component.literal(plot.summary()), x0 + 6, top + 10, PANEL_W - 12, TEXT);
            return;
        }
        // 2:1 band shading
        if (Double.isFinite(plot.bandLowHz()) && Double.isFinite(plot.bandHighHz())) {
            int a = left + (int) Math.round(clamp01(sp.xFracOf(plot.bandLowHz())) * (w - 1));
            int b = left + (int) Math.round(clamp01(sp.xFracOf(plot.bandHighHz())) * (w - 1));
            if (b > a) g.fill(a, top, b + 1, top + h, BAND);
        }
        // grid + SWR labels
        for (double s : SwrPlot.GRID) {
            int r = SwrPlot.row(s, top, h);
            g.hLine(left, left + w - 1, r, s == 2 ? LINE_2TO1 : GRID);
            g.drawString(font, fmt(s), left - 4 - font.width(fmt(s)), r - 4, s == 2 ? LINE_2TO1 : DIM, false);
        }
        g.drawString(font, "1", left - 4 - font.width("1"), top + h - 8, DIM, false);
        g.drawString(font, "10+", left - 4 - font.width("10+"), top, DIM, false);
        double lo = plot.hz()[0], hi = plot.hz()[plot.hz().length - 1];
        List<Double> ticks = SwrPlot.ticks(lo, hi, 6);
        for (double f : ticks) {
            int c = left + (int) Math.round(sp.xFracOf(f) * (w - 1));
            g.vLine(c, top - 1, top + h, GRID);
            String l = SwrPlot.label(f);
            g.drawString(font, l, c - font.width(l) / 2, top + h + 3, DIM, false);
        }
        g.drawString(font, SwrPlot.unit(lo), left + w - font.width(SwrPlot.unit(lo)), top + h + 13, DIM, false);
        g.renderOutline(left - 1, top - 1, w + 2, h + 2, FRAME);
        // resonance marker
        if (Double.isFinite(plot.resonantHz())) {
            double fx = sp.xFracOf(plot.resonantHz());
            if (fx >= 0 && fx <= 1) g.vLine(left + (int) Math.round(fx * (w - 1)), top, top + h - 1, 0x80FFFFFF);
        }
        // curve
        int pc = -1, pr = -1;
        for (int i = 0; i < plot.hz().length; i++) {
            int c = sp.col(i, left, w), r = SwrPlot.row(plot.swr()[i], top, h);
            if (pc >= 0) line(g, pc, pr, c, r, CURVE);
            pc = c;
            pr = r;
        }
        int m = sp.minIndex();
        if (m >= 0) {
            int c = sp.col(m, left, w), r = SwrPlot.row(plot.swr()[m], top, h);
            g.fill(c - 2, r - 2, c + 3, r + 3, MARK);
            String best = String.format(Locale.ROOT, "%.2f:1 @ %s %s", plot.swr()[m], SwrPlot.label(plot.hz()[m]), SwrPlot.unit(plot.hz()[m]));
            g.drawString(font, best, left + w - font.width(best) - 2, top + 2, MARK, false);
        }
        int ty = top + h + 24;
        for (var line : font.split(Component.literal(plot.summary()), PANEL_W - 12)) {
            g.drawString(font, line, x0 + 6, ty, TEXT, false);
            ty += 10;
            if (ty > y0 + PANEL_H - 20) break;
        }
        if (!plot.limits().isEmpty()) {
            for (var line : font.split(Component.literal(plot.limits()), PANEL_W - 12)) {
                if (ty > y0 + PANEL_H - 10) break;
                g.drawString(font, line, x0 + 6, ty, DIM, false);
                ty += 10;
            }
        }
    }

    private static double clamp01(double v) {
        return Math.max(0, Math.min(1, v));
    }

    private static String fmt(double s) {
        return s == Math.rint(s) ? String.valueOf((int) s) : String.valueOf(s);
    }

    /** A one-pixel line (Bresenham). */
    private static void line(GuiGraphics g, int x0, int y0, int x1, int y1, int color) {
        int dx = Math.abs(x1 - x0), dy = -Math.abs(y1 - y0), sx = x0 < x1 ? 1 : -1, sy = y0 < y1 ? 1 : -1, err = dx + dy;
        while (true) {
            g.fill(x0, y0, x0 + 1, y0 + 1, color);
            if (x0 == x1 && y0 == y1) break;
            int e2 = 2 * err;
            if (e2 >= dy) { err += dy; x0 += sx; }
            if (e2 <= dx) { err += dx; y0 += sy; }
        }
    }
}
//?}
