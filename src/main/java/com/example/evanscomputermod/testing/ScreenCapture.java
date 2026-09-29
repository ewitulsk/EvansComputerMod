package com.example.evanscomputermod.testing;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.computer.TerminalDisplay;

import javax.imageio.ImageIO;
import java.awt.Color;
import java.awt.Font;
import java.awt.Graphics2D;
import java.awt.RenderingHints;
import java.awt.image.BufferedImage;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

/**
 * Renders terminal framebuffers to PNG, side by side, for test evidence
 * ("server" vs "client" screens). Rows whose text differs from the first
 * panel are marked red. Output goes to {@code <game dir>/screenshots/}, which
 * scripts/Test.ps1 copies into the run's artifacts.
 */
public final class ScreenCapture {
    public record Panel(String label, TerminalDisplay display) {}

    private static final int CW = 8, CH = 16, PAD = 12, HEADER = 22;
    /** VGA-style 16-colour palette for the low nibble of a cell's attribute. */
    private static final int[] VGA = {
            0x000000, 0x0000AA, 0x00AA00, 0x00AAAA, 0xAA0000, 0xAA00AA, 0xAA5500, 0xAAAAAA,
            0x555555, 0x5555FF, 0x55FF55, 0x55FFFF, 0xFF5555, 0xFF55FF, 0xFFFF55, 0xFFFFFF};

    public static Path dir() {
        return Path.of(System.getProperty("ecm.screenshotDir", "screenshots"));
    }

    /** Write {@code panels} to {@code <dir>/<name>.png}; returns the file, or null on error. */
    public static Path write(String name, String title, List<Panel> panels) {
        System.setProperty("java.awt.headless", "true");
        try {
            int maxW = 0, maxH = 0;
            for (Panel p : panels) {
                maxW = Math.max(maxW, p.display().getWidth());
                maxH = Math.max(maxH, p.display().getHeight());
            }
            int panelW = maxW * CW, panelH = maxH * CH;
            BufferedImage img = new BufferedImage(PAD + panels.size() * (panelW + PAD),
                    HEADER * 2 + panelH + PAD, BufferedImage.TYPE_INT_RGB);
            Graphics2D g = img.createGraphics();
            g.setRenderingHint(RenderingHints.KEY_TEXT_ANTIALIASING, RenderingHints.VALUE_TEXT_ANTIALIAS_ON);
            g.setColor(new Color(0x202020));
            g.fillRect(0, 0, img.getWidth(), img.getHeight());
            g.setFont(new Font(Font.SANS_SERIF, Font.BOLD, 13));
            g.setColor(Color.WHITE);
            g.drawString(title, PAD, 16);

            List<String> reference = rows(panels.get(0).display());
            Font mono = new Font(Font.MONOSPACED, Font.PLAIN, 14);
            for (int i = 0; i < panels.size(); i++) {
                Panel p = panels.get(i);
                TerminalDisplay d = p.display();
                int x0 = PAD + i * (panelW + PAD), y0 = HEADER * 2;
                g.setFont(new Font(Font.SANS_SERIF, Font.BOLD, 13));
                g.setColor(new Color(0xDDDDDD));
                g.drawString(p.label(), x0, HEADER + 16);
                g.setColor(Color.BLACK);
                g.fillRect(x0, y0, panelW, panelH);
                List<String> mine = rows(d);
                g.setFont(mono);
                for (int y = 0; y < d.getHeight(); y++) {
                    if (i > 0 && (y >= reference.size() || !mine.get(y).equals(reference.get(y)))) {
                        g.setColor(new Color(0x5A0000)); // differs from the first panel
                        g.fillRect(x0, y0 + y * CH, panelW, CH);
                    }
                    for (int x = 0; x < d.getWidth(); x++) {
                        int c = d.getCharAt(x, y) & 0xFF;
                        if (c <= 32 || c >= 127) continue;
                        g.setColor(new Color(VGA[d.getAttrAt(x, y) & 0x0F]));
                        g.drawString(String.valueOf((char) c), x0 + x * CW, y0 + y * CH + 12);
                    }
                }
                if (d.isCursorVisible() && d.getCursorY() < d.getHeight()) {
                    g.setColor(new Color(0x55FF55));
                    g.fillRect(x0 + d.getCursorX() * CW, y0 + d.getCursorY() * CH + 13, CW, 2);
                }
            }
            g.dispose();
            Path out = dir().resolve(name + ".png");
            Files.createDirectories(out.getParent());
            ImageIO.write(img, "png", out.toFile());
            EvansComputerMod.LOGGER.info("ECM_SCREENSHOT {}", out.toAbsolutePath());
            return out;
        } catch (IOException | RuntimeException e) {
            EvansComputerMod.LOGGER.warn("screenshot {} failed", name, e);
            return null;
        }
    }

    static List<String> rows(TerminalDisplay d) {
        return List.of(com.example.evanscomputermod.testing.scenario.ScenarioRun.screen(d).split("\n", -1));
    }

    private ScreenCapture() {}
}
