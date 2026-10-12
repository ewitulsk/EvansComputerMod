package com.example.evanscomputermod.testing.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.TerminalScreen;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.testing.RadioVisualLayout;
import com.mojang.blaze3d.platform.NativeImage;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Screenshot;
import net.neoforged.neoforge.client.event.ClientTickEvent;

import java.nio.file.Files;
import java.nio.file.Path;

/**
 * GUI render suite, hidden-client half: with the Terminal's screen open on the GUI, checks
 * the client's display carries the composited desktop (640x400 RGBA), screenshots it and
 * samples the shell's bar and the gba window in the real render; then clicks Notes' text
 * field and types through the screen's own input handlers (the same calls Minecraft's input
 * makes), waits for the server to confirm the click focused Notes and the text reached it,
 * and screenshots that. Then (P3) the server restarts the GUI with the desktop shell: the
 * client screenshots the desktop and samples its taskbar, clicks the Apps button through the
 * screen, waits for the server to see the launcher popup, and screenshots that.
 */
public final class GuiClientChecks {
    private static final int SETTLE = 10;
    private static final long TIMEOUT_MS = 150_000;
    private static volatile int targetX = -1, targetY = -1, gbaX = -1, gbaY = -1;
    private static volatile boolean typedConfirmed, launcherConfirmed;
    private static volatile int appsX = -1, appsY = -1;
    private static int settled, step, stopTicks = -1, wait;
    private static long started;

    private GuiClientChecks() {}

    public static void onChat(String text) {
        if (!RadioVisualLayout.active("gui")) return;
        if (text.startsWith("ECMGUI ready ")) {
            String[] p = text.split(" ");
            targetX = Integer.parseInt(p[2]);
            targetY = Integer.parseInt(p[3]);
            gbaX = Integer.parseInt(p[4]);
            gbaY = Integer.parseInt(p[5]);
        } else if (text.equals("ECMGUI typed")) {
            typedConfirmed = true;
        } else if (text.startsWith("ECMGUI desktop ")) {
            String[] p = text.split(" ");
            appsX = Integer.parseInt(p[2]);
            appsY = Integer.parseInt(p[3]);
        } else if (text.equals("ECMGUI launcher")) {
            launcherConfirmed = true;
        }
    }

    public static void tick(ClientTickEvent.Post event) {
        if (!RadioVisualLayout.active("gui")) return;
        var mc = Minecraft.getInstance();
        if (stopTicks >= 0) {
            if (--stopTicks == 0) mc.stop();
            return;
        }
        if (mc.level == null || mc.player == null || step < 0) return;
        if (started == 0) started = System.currentTimeMillis();
        try {
            if (System.currentTimeMillis() - started > TIMEOUT_MS) throw new IllegalStateException("GUI render checks exceeded " + TIMEOUT_MS / 1000 + " s");
            mc.options.pauseOnLostFocus = false;
            if (!(mc.screen instanceof TerminalScreen screen) || targetX < 0) {
                settled = 0;
                return;
            }
            TerminalDisplay d = screen.getMenu().getBlockEntity().getDisplay();
            float[] r = screen.gfxRect();
            if (r == null || d.getDisplayMode() != 1 || d.getGfxWidth() != 640 || d.getGfxHeight() != 400
                    || d.getPixelFormat() != TerminalDisplay.PIXEL_FORMAT_RGBA8888) {
                settled = 0;
                return;
            }
            switch (step) {
                case 0 -> {
                    if (++settled < SETTLE) return;
                    try (NativeImage img = shot(mc, "gui_desktop.png")) {
                        double gs = mc.getWindow().getGuiScale();
                        int bar = max(img, at(r, 630, 6), gs);
                        int barY = maxY(img, at(r, 630, 6), gs);
                        int gba = 0;
                        for (int dx = -40; dx <= 40; dx += 20) gba = Math.max(gba, max(img, at(r, gbaX + dx, gbaY - 30), gs));
                        EvansComputerMod.LOGGER.info("ECM_VISUAL_GUI_SAMPLES bar max={} at y={}; gba window max={}", bar, barY, gba);
                        if (bar > 90) throw new IllegalStateException("the shell's bar isn't drawn dark at the top: " + bar);
                        if (gba < 180) throw new IllegalStateException("the gba window (white test ROM screen) isn't drawn: " + gba);
                    }
                    EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS gui_desktop client display 640x400 RGBA, desktop captured");
                    step = 1;
                    wait = 0;
                }
                case 1 -> {
                    // Click Notes' text field (framebuffer pixels -> GUI coordinates).
                    double gx = r[0] + (targetX + 0.5) * (r[2] - r[0]) / 640.0;
                    double gy = r[1] + (targetY + 0.5) * (r[3] - r[1]) / 400.0;
                    screen.mouseMoved(gx, gy);
                    screen.mouseClicked(gx, gy, 0);
                    screen.mouseReleased(gx, gy, 0);
                    step = 2;
                    wait = 0;
                }
                case 2 -> {
                    if (++wait < SETTLE) return; // let the shell focus Notes
                    for (char c : "hello gui".toCharArray()) screen.charTyped(c, 0);
                    step = 3;
                    settled = 0;
                }
                case 3 -> {
                    if (!typedConfirmed || ++settled < SETTLE) return;
                    try (NativeImage img = shot(mc, "gui_typed.png")) {
                        EvansComputerMod.LOGGER.info("ECM_VISUAL_GUI_TYPED {}x{}", img.getWidth(), img.getHeight());
                    }
                    EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS gui_typed click and text reached Notes");
                    step = 4;
                    settled = 0;
                }
                case 4 -> {
                    // The desktop shell (P3): wait for the server, then let the frames settle.
                    if (appsX < 0 || ++settled < SETTLE * 2) return;
                    try (NativeImage img = shot(mc, "gui_p3_desktop.png")) {
                        double gs = mc.getWindow().getGuiScale();
                        int taskbar = max(img, at(r, 320, 395), gs);
                        int apps = max(img, at(r, appsX, appsY - 6), gs);
                        EvansComputerMod.LOGGER.info("ECM_VISUAL_GUI_SAMPLES taskbar max={} apps button max={}", taskbar, apps);
                        if (taskbar > 90) throw new IllegalStateException("the desktop's taskbar isn't drawn dark at the bottom: " + taskbar);
                        if (apps < 120) throw new IllegalStateException("the Apps button (accent blue) isn't drawn: " + apps);
                    }
                    EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS gui_p3_desktop the desktop, Settings and Terminal captured");
                    step = 5;
                }
                case 5 -> {
                    double gx = r[0] + (appsX + 0.5) * (r[2] - r[0]) / 640.0;
                    double gy = r[1] + (appsY + 0.5) * (r[3] - r[1]) / 400.0;
                    screen.mouseMoved(gx, gy);
                    screen.mouseClicked(gx, gy, 0);
                    screen.mouseReleased(gx, gy, 0);
                    step = 6;
                    settled = 0;
                }
                case 6 -> {
                    if (!launcherConfirmed || ++settled < SETTLE) return;
                    try (NativeImage img = shot(mc, "gui_p3_launcher.png")) {
                        EvansComputerMod.LOGGER.info("ECM_VISUAL_GUI_LAUNCHER {}x{}", img.getWidth(), img.getHeight());
                    }
                    EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS gui_p3_launcher the click opened the launcher");
                    mc.player.connection.sendCommand("ecmvisual gui finish");
                    step = -1;
                    stopTicks = 40;
                }
                default -> { }
            }
        } catch (Throwable t) {
            EvansComputerMod.LOGGER.error("ECM_VISUAL_CLIENT_FAIL", t);
            step = -1;
            mc.stop();
        }
    }

    private static NativeImage shot(Minecraft mc, String name) throws Exception {
        Path dir = Path.of(System.getProperty("ecm.visualOutput"));
        Files.createDirectories(dir);
        NativeImage img = Screenshot.takeScreenshot(mc.getMainRenderTarget());
        img.writeToFile(dir.resolve(name));
        return img;
    }

    /** GUI coordinates of framebuffer pixel (x, y). */
    private static double[] at(float[] r, int x, int y) {
        return new double[]{r[0] + (x + 0.5) * (r[2] - r[0]) / 640.0, r[1] + (y + 0.5) * (r[3] - r[1]) / 400.0};
    }

    private static int max(NativeImage img, double[] p, double gs) {
        int x = (int) Math.round(p[0] * gs), y = (int) Math.round(p[1] * gs);
        int abgr = img.getPixelRGBA(Math.min(img.getWidth() - 1, x), Math.min(img.getHeight() - 1, y));
        return Math.max(abgr & 0xFF, Math.max((abgr >> 8) & 0xFF, (abgr >> 16) & 0xFF));
    }

    private static int maxY(NativeImage img, double[] p, double gs) {
        return (int) Math.round(p[1] * gs);
    }
}
//?}
