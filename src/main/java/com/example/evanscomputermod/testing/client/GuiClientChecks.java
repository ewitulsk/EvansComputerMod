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
 * and screenshots that.
 *
 * <p>With client rendering on, the display is drawn by this client from the scene stream
 * ({@code ClientUiScenes}); the check requires that, and that Notes' window in it has the
 * same CRC as in the server's frame (sent with the typed confirmation).
 *
 * <p>Then (P3), once the client asks for it ({@code /ecmvisual gui desktop}), the server
 * restarts the GUI with the desktop shell: the client screenshots the desktop and samples its
 * taskbar, clicks the Apps button through the screen, waits for the server to see the
 * launcher popup, and screenshots that.
 *
 * <p>Then Esc, through Minecraft's own keyboard handler: Shift+Esc keeps the screen open (it
 * goes to the computer); Esc closes the Terminal screen on the GUI.
 * The server opens it again, Ctrl+Alt+Backspace switches to the console, Shift+Esc keeps the
 * screen open there too (control), and Esc closes it. Each close is checked on both sides.
 */
public final class GuiClientChecks {
    private static final int SETTLE = 10;
    private static final long TIMEOUT_MS = 150_000;
    private static volatile int targetX = -1, targetY = -1, gbaX = -1, gbaY = -1;
    private static volatile boolean typedConfirmed, launcherConfirmed;
    private static volatile int appsX = -1, appsY = -1;
    private static volatile int closeX = -1, closeY = -1;
    private static volatile boolean closeConfirmed;
    /** Esc checks: the server saw the menu stay open (Shift+Esc), the menu close on the GUI, reopened it, and the menu close on the console. */
    private static volatile boolean popupClosed, escGuiClosed, reopened, escConsoleClosed;
    private static int escWait;
    /** The simulated overlay: consumes screen clicks at x >= overlayFromX (GUI units) at normal priority. */
    private static volatile double overlayFromX = Double.MAX_VALUE;
    private static volatile int overlayHits;
    private static boolean overlayInstalled;
    private static int dispatchedBefore;
    private static volatile boolean focusConfirmed;
    private static volatile long serverCrc = -1;
    private static volatile int[] notesRect;
    private static int crcTries;
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
        } else if (text.equals("ECMGUI focused")) {
            focusConfirmed = true;
        } else if (text.equals("ECMGUI typed")) {
            typedConfirmed = true;
        } else if (text.startsWith("ECMGUI desktop ")) {
            String[] p = text.split(" ");
            appsX = Integer.parseInt(p[2]);
            appsY = Integer.parseInt(p[3]);
        } else if (text.startsWith("ECMGUI close ")) {
            String[] p = text.split(" ");
            closeX = Integer.parseInt(p[2]);
            closeY = Integer.parseInt(p[3]);
        } else if (text.equals("ECMGUI closed")) {
            closeConfirmed = true;
        } else if (text.equals("ECMGUI launcher")) {
            launcherConfirmed = true;
        } else if (text.equals("ECMGUI popup_closed")) {
            popupClosed = true;
        } else if (text.equals("ECMGUI esc_gui")) {
            escGuiClosed = true;
        } else if (text.equals("ECMGUI reopened")) {
            reopened = true;
        } else if (text.equals("ECMGUI esc_console")) {
            escConsoleClosed = true;
        } else if (text.startsWith("ECMGUI crc ")) {
            String[] p = text.split(" ");
            notesRect = new int[]{Integer.parseInt(p[3]), Integer.parseInt(p[4]), Integer.parseInt(p[5]), Integer.parseInt(p[6])};
            serverCrc = Long.parseLong(p[2]);
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
            if (step >= 7 && step <= 11) {
                escStep(mc);
                return;
            }
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
                    if (!focusConfirmed || ++wait < SETTLE) return; // the shell focused Notes (server confirms)
                    for (char c : "hello gui".toCharArray()) screen.charTyped(c, 0);
                    step = 3;
                    settled = 0;
                }
                case 3 -> {
                    // The server sees Notes focused and changed; give Notes time to draw the typed text too.
                    if (!typedConfirmed || ++settled < SETTLE * 6) return;
                    java.util.UUID id = screen.getMenu().getBlockEntity().getComputerId();
                    boolean sceneCase = com.example.evanscomputermod.client.ClientUiSceneHooks.enabled();
                    if (sceneCase && !com.example.evanscomputermod.computer.ui.ClientUiScenes.active(id, 0)) {
                        throw new IllegalStateException("the client isn't drawing the GUI from the scene stream");
                    }
                    // Ask the server for Notes' CRC in its frame now; the scene may lag a little: ask again.
                    if (sceneCase && (serverCrc < 0 || notesRect == null)) {
                        if (crcTries++ % 20 == 0) mc.player.connection.sendCommand("ecmvisual gui crc");
                        if (crcTries > 400) throw new IllegalStateException("no CRC from the server");
                        return;
                    }
                    if (sceneCase) {
                        long mine;
                        synchronized (d) {
                            mine = com.example.evanscomputermod.testing.GuiServerChecks.crc(d.getPixelData(), notesRect[0], notesRect[1],
                                    notesRect[2], notesRect[3]);
                        }
                        // A blinking caret or a patch in flight may differ for a moment: ask again for a while.
                        if (mine != serverCrc) {
                            if (crcTries > 300) {
                                throw new IllegalStateException("Notes' window differs from the server's frame (crc "
                                        + mine + " vs " + serverCrc + ")");
                            }
                            serverCrc = -1;
                            return;
                        }
                        try (NativeImage img = shot(mc, "gui_client_render.png")) {
                            EvansComputerMod.LOGGER.info("ECM_VISUAL_GUI_CLIENT_RENDER {}x{}", img.getWidth(), img.getHeight());
                        }
                        var m = com.example.evanscomputermod.computer.ui.ClientUiScenes.get(id, 0);
                        EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS gui_client_render the client draws the GUI itself ({} renders, {} scene messages, {} B); Notes' window matches the server's frame (crc {})",
                                m.renders(), com.example.evanscomputermod.computer.ui.ClientUiScenes.messagesReceived(),
                                com.example.evanscomputermod.computer.ui.ClientUiScenes.bytesReceived(), mine);
                    }
                    try (NativeImage img = shot(mc, "gui_typed.png")) {
                        EvansComputerMod.LOGGER.info("ECM_VISUAL_GUI_TYPED {}x{}", img.getWidth(), img.getHeight());
                    }
                    EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS gui_typed click and text reached Notes");
                    // Done with Notes (and its CRC): the server may switch to the desktop shell now.
                    mc.player.connection.sendCommand("ecmvisual gui desktop");
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
                    mc.player.connection.sendCommand("ecmvisual gui close");
                    step = 40;
                    wait = 0;
                }
                case 40 -> {
                    // A window's close button under an overlay (JEI's ingredient list, simulated): a listener at
                    // normal priority that consumes every screen click right of the button. A real click, through
                    // Minecraft's mouse handler (the events fire), must still reach the computer.
                    if (closeX < 0) return;
                    double gx = r[0] + (closeX + 0.5) * (r[2] - r[0]) / 640.0;
                    double gy = r[1] + (closeY + 0.5) * (r[3] - r[1]) / 400.0;
                    installOverlay();
                    overlayFromX = gx - 20;
                    int guiW = mc.getWindow().getGuiScaledWidth(), guiH = mc.getWindow().getGuiScaledHeight();
                    EvansComputerMod.LOGGER.info("ECM_VISUAL_GUI_OVERLAY close button at gui {},{} of {}x{}; overlay consumes x >= {}",
                            String.format("%.1f", gx), String.format("%.1f", gy), guiW, guiH, String.format("%.1f", overlayFromX));
                    // Control: a click in the overlay beside the Terminal (outside ECM's area) is the overlay's.
                    dispatchedBefore = com.example.evanscomputermod.client.ComputerScreenInput.dispatched();
                    realClick(mc, guiW - 2, guiH / 2.0);
                    if (overlayHits != 2) throw new IllegalStateException("control: the overlay got " + overlayHits + " of the press and release beside the Terminal");
                    if (com.example.evanscomputermod.client.ComputerScreenInput.dispatched() != dispatchedBefore)
                        throw new IllegalStateException("control: a click outside the Terminal went to the computer");
                    // The close button, under the overlay (the pointer moves there first, as a real one does).
                    screen.mouseMoved(gx, gy);
                    realClick(mc, gx, gy);
                    int got = com.example.evanscomputermod.client.ComputerScreenInput.dispatched() - dispatchedBefore;
                    if (overlayHits != 2) throw new IllegalStateException("the overlay consumed the click on the close button (" + overlayHits + " hits)");
                    if (got != 2) throw new IllegalStateException("the computer screen got " + got + " of the press and release");
                    step = 41;
                    settled = 0;
                }
                case 41 -> {
                    if (!closeConfirmed || ++settled < SETTLE) return;
                    try (NativeImage img = shot(mc, "gui_close_over_overlay.png")) {
                        EvansComputerMod.LOGGER.info("ECM_VISUAL_GUI_CLOSED {}x{}", img.getWidth(), img.getHeight());
                    }
                    overlayFromX = Double.MAX_VALUE;
                    EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS gui_close_over_overlay the close button under a click-eating overlay closed the window (overlay hits {}: the control only)",
                            overlayHits);
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
                    // Shift+Esc is Esc for the computer: the screen stays open.
                    realKey(mc, org.lwjgl.glfw.GLFW.GLFW_KEY_ESCAPE, org.lwjgl.glfw.GLFW.GLFW_MOD_SHIFT);
                    step = 7;
                    escWait = 0;
                }
                default -> { }
            }
        } catch (Throwable t) {
            EvansComputerMod.LOGGER.error("ECM_VISUAL_CLIENT_FAIL", t);
            step = -1;
            mc.stop();
        }
    }

    /** The stand-in for an overlay mod: consumes clicks right of overlayFromX, at normal priority, like JEI's listeners. */
    private static void installOverlay() {
        if (overlayInstalled) return;
        overlayInstalled = true;
        net.neoforged.neoforge.common.NeoForge.EVENT_BUS.addListener(net.neoforged.bus.api.EventPriority.NORMAL, false,
                net.neoforged.neoforge.client.event.ScreenEvent.MouseButtonPressed.Pre.class, e -> {
                    if (e.getMouseX() >= overlayFromX) {
                        overlayHits++;
                        e.setCanceled(true);
                    }
                });
        net.neoforged.neoforge.common.NeoForge.EVENT_BUS.addListener(net.neoforged.bus.api.EventPriority.NORMAL, false,
                net.neoforged.neoforge.client.event.ScreenEvent.MouseButtonReleased.Pre.class, e -> {
                    if (e.getMouseX() >= overlayFromX) {
                        overlayHits++;
                        e.setCanceled(true);
                    }
                });
    }

    /**
     * The Esc checks (steps 7 to 11): every key goes through Minecraft's keyboard handler, the
     * way a player's key does, and the server confirms what the computer and its menu saw.
     */
    private static void escStep(Minecraft mc) throws Exception {
        int esc = org.lwjgl.glfw.GLFW.GLFW_KEY_ESCAPE;
        switch (step) {
            case 7 -> {
                if (!(mc.screen instanceof TerminalScreen)) throw new IllegalStateException("Shift+Esc closed the Terminal screen on the GUI");
                if (!popupClosed || ++escWait < SETTLE) return;
                EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS gui_shift_esc Shift+Esc on the GUI kept the screen open");
                realKey(mc, esc, 0);
                step = 8;
                escWait = 0;
            }
            case 8 -> {
                if (mc.screen instanceof TerminalScreen) {
                    if (++escWait > 40) throw new IllegalStateException("Esc didn't close the Terminal screen on the GUI");
                    return;
                }
                if (!escGuiClosed) return;
                EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS gui_esc_closes Esc closed the Terminal screen on the GUI (client and server)");
                step = 9;
                escWait = 0;
            }
            case 9 -> {
                // The server opened the screen again: Ctrl+Alt+Backspace switches to the console.
                if (!reopened || !(mc.screen instanceof TerminalScreen screen)) return;
                if (++escWait == SETTLE) {
                    realKey(mc, org.lwjgl.glfw.GLFW.GLFW_KEY_BACKSPACE, org.lwjgl.glfw.GLFW.GLFW_MOD_CONTROL | org.lwjgl.glfw.GLFW.GLFW_MOD_ALT);
                }
                if (escWait < SETTLE || screen.getMenu().getBlockEntity().getDisplay().getDisplayMode() != 0) {
                    if (escWait > 400) throw new IllegalStateException("Ctrl+Alt+Backspace didn't switch the Terminal to the console");
                    return;
                }
                realKey(mc, esc, org.lwjgl.glfw.GLFW.GLFW_MOD_SHIFT);
                step = 10;
                escWait = 0;
            }
            case 10 -> {
                if (!(mc.screen instanceof TerminalScreen)) throw new IllegalStateException("Shift+Esc closed the Terminal screen on the console");
                if (++escWait < SETTLE) return;
                EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS console_shift_esc control: Shift+Esc on the console kept the screen open");
                realKey(mc, esc, 0);
                step = 11;
                escWait = 0;
            }
            case 11 -> {
                if (mc.screen instanceof TerminalScreen) {
                    if (++escWait > 40) throw new IllegalStateException("Esc didn't close the Terminal screen on the console");
                    return;
                }
                if (!escConsoleClosed) return;
                EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS console_esc_closes Esc closed the Terminal screen on the console (client and server)");
                mc.player.connection.sendCommand("ecmvisual gui finish");
                step = -1;
                stopTicks = 40;
            }
            default -> { }
        }
    }

    private static java.lang.reflect.Method keyPress;

    /** A key press and release the way GLFW delivers them: the keyboard handler's own callback (NeoForge's screen events fire). */
    private static void realKey(Minecraft mc, int key, int mods) throws Exception {
        if (keyPress == null) {
            keyPress = net.minecraft.client.KeyboardHandler.class.getDeclaredMethod("keyPress", long.class, int.class, int.class, int.class, int.class);
            keyPress.setAccessible(true);
        }
        long win = mc.getWindow().getWindow();
        keyPress.invoke(mc.keyboardHandler, win, key, 0, org.lwjgl.glfw.GLFW.GLFW_PRESS, mods);
        keyPress.invoke(mc.keyboardHandler, win, key, 0, org.lwjgl.glfw.GLFW.GLFW_RELEASE, mods);
    }

    private static java.lang.reflect.Method onMove, onPress;

    /**
     * A left click at GUI point ({@code gx}, {@code gy}) the way GLFW delivers one: the mouse
     * handler's own move and press/release callbacks (so NeoForge's screen events fire, and
     * every listener sees them in priority order), not the screen's methods.
     */
    private static void realClick(Minecraft mc, double gx, double gy) throws Exception {
        if (onMove == null) {
            onMove = net.minecraft.client.MouseHandler.class.getDeclaredMethod("onMove", long.class, double.class, double.class);
            onPress = net.minecraft.client.MouseHandler.class.getDeclaredMethod("onPress", long.class, int.class, int.class, int.class);
            onMove.setAccessible(true);
            onPress.setAccessible(true);
        }
        long win = mc.getWindow().getWindow();
        double sx = gx * mc.getWindow().getScreenWidth() / mc.getWindow().getGuiScaledWidth();
        double sy = gy * mc.getWindow().getScreenHeight() / mc.getWindow().getGuiScaledHeight();
        onMove.invoke(mc.mouseHandler, win, sx, sy);
        onMove.invoke(mc.mouseHandler, win, sx, sy);   // the first move after setup is only remembered
        onPress.invoke(mc.mouseHandler, win, 0, 1, 0);
        onPress.invoke(mc.mouseHandler, win, 0, 0, 0);
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
