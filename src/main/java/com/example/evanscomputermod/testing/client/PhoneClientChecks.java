package com.example.evanscomputermod.testing.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.network.ClientDisplayCache;
import com.example.evanscomputermod.phone.PhoneItemData;
import com.example.evanscomputermod.phone.PhoneState;
import com.example.evanscomputermod.phone.client.PhoneClient;
import com.example.evanscomputermod.phone.client.PhoneItemRenderer;
import com.example.evanscomputermod.testing.RadioVisualLayout;
import com.example.evanscomputermod.testing.scenario.ScenarioRun;
import com.mojang.blaze3d.platform.InputConstants;
import com.mojang.blaze3d.platform.NativeImage;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Screenshot;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.client.event.ClientTickEvent;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.UUID;

/**
 * Phone render suite, hidden-client half (P5b): the phone in the main hand and
 * then the off hand must show the phone shell (the renderer drew the display
 * texture, the display is the 240x400 GUI drawn here from the scene stream,
 * and the photo has the home screen's teal wallpaper where the phone is);
 * opened, a click on Notes opens it (the server says so); then with hotkey
 * slot 1 bound to W, holding W both walks (the player moves) and reaches the
 * open app (the server reports the Counter's "Bump").
 */
public final class PhoneClientChecks {
    private static final long TIMEOUT_MS = 180_000;
    private static final int SETTLE = 20;
    private static int phase, settled, walkTicks, stopTicks = -1, sentBefore;
    private static long started;
    private static Vec3 walkFrom;

    private PhoneClientChecks() {}

    public static void tick(ClientTickEvent.Post event) {
        if (!RadioVisualLayout.active("phone")) return;
        Minecraft mc = Minecraft.getInstance();
        if (stopTicks >= 0) {
            if (--stopTicks == 0) mc.stop();
            return;
        }
        if (mc.level == null || mc.player == null) return;
        if (started == 0) started = System.currentTimeMillis();
        try {
            if (System.currentTimeMillis() - started > TIMEOUT_MS) throw new IllegalStateException("phone suite timed out in phase " + phase);
            mc.options.pauseOnLostFocus = false;
            mc.options.bobView().set(false);
            switch (phase) {
                case 0 -> mainHand(mc);
                case 1 -> offHand(mc);
                case 2 -> open(mc);
                case 3 -> walk(mc);
                case 4 -> dock(mc);
                default -> {
                }
            }
        } catch (Throwable t) {
            EvansComputerMod.LOGGER.error("ECM_VISUAL_CLIENT_FAIL", t);
            mc.stop();
        }
    }

    private static int debugTicks;

    private static boolean showing(UUID id, String last) {
        PhoneClient.Status st = PhoneClient.status(id);
        var e = ClientDisplayCache.entry(id, 0);
        if (++debugTicks % 100 == 0) {
            EvansComputerMod.LOGGER.info("ECM_VISUAL_PHONE_WAIT phase={} status={} keyframe={} screenHasText={} drew={}", phase, st,
                    e.hasKeyframe, e.sceneDriven, last);
            if (debugTicks % 500 == 0) EvansComputerMod.LOGGER.info("ECM_VISUAL_PHONE_SCREEN {}x{}\n{}", e.display.getWidth(),
                    e.display.getHeight(), ScenarioRun.screen(e.display));
        }
        // P5b: the phone shell, drawn on this client from the scene stream (client rendering) at 240x400.
        return st != null && st.state() == PhoneState.AWAKE && e.sceneDriven && e.display.getDisplayMode() == 1
                && e.display.getGfxWidth() == 240 && e.display.getGfxHeight() == 400 && last.startsWith("display:");
    }

    private static void mainHand(Minecraft mc) throws Exception {
        UUID id = PhoneItemData.id(mc.player.getMainHandItem());
        if (id == null || !showing(id, PhoneItemRenderer.lastFirstPerson)) {
            settled = 0;
            return;
        }
        if (++settled < SETTLE) return;
        int teal = capture(mc, "phone_main_hand", true);
        if (teal < 40) throw new IllegalStateException("main hand: only " + teal + " wallpaper pixels where the phone is");
        EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS phone_main_hand teal={} drew={}", teal, PhoneItemRenderer.lastFirstPerson);
        mc.player.connection.sendCommand("ecmvisual phone offhand");
        phase = 1;
        settled = 0;
    }

    private static void offHand(Minecraft mc) throws Exception {
        UUID id = PhoneItemData.id(mc.player.getOffhandItem());
        if (id == null || !mc.player.getMainHandItem().isEmpty() || !showing(id, PhoneItemRenderer.lastOffHand)) {
            settled = 0;
            return;
        }
        if (++settled < SETTLE) return;
        int teal = capture(mc, "phone_off_hand", false);
        if (teal < 40) throw new IllegalStateException("off hand: only " + teal + " wallpaper pixels where the phone is");
        EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS phone_off_hand teal={} drew={}", teal, PhoneItemRenderer.lastOffHand);
        phase = 2;
        settled = 0;
        openTicks = 0;
    }

    private static int openTicks;

    /** Open the phone full screen and click Notes on the home grid (the mouse as a finger). */
    private static void open(Minecraft mc) throws Exception {
        openTicks++;
        if (openTicks == 1) {
            com.example.evanscomputermod.phone.client.PhoneScreen.open(net.minecraft.world.InteractionHand.OFF_HAND);
            return;
        }
        if (openTicks > 90) {
            if (openTicks < 130) return;
            // Hotkey slot 1 on W, the walk-forward key: both must work.
            PhoneClient.SLOTS[0].setKey(InputConstants.getKey("key.keyboard.w"));
            KeyMapping.resetMapping();
            sentBefore = PhoneClient.hotkeysSent();
            walkFrom = mc.player.position();
            walkTicks = 0;
            phase = 3;
            return;
        }
        if (!(mc.screen instanceof com.example.evanscomputermod.phone.client.PhoneScreen ps)) throw new IllegalStateException("the phone didn't open");
        if (openTicks == 30) {
            // Notes is the second app on the grid (GBA, Notes, Controls): its icon at phone pixel (90, 42).
            double[] at = ps.screenPoint(90, 42);
            ps.mouseMoved(at[0], at[1]);
            ps.mouseClicked(at[0], at[1], 0);
            ps.mouseReleased(at[0], at[1], 0);
            return;
        }
        if (openTicks == 90) {
            capture(mc, "phone_open", true);
            EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS phone_open keys={}", ps.keysSent());
            mc.setScreen(null);
            mc.player.connection.sendCommand("ecmvisual phone app");
        }
    }

    private static void walk(Minecraft mc) throws Exception {
        InputConstants.Key w = InputConstants.getKey("key.keyboard.w");
        walkTicks++;
        if (walkTicks <= 30) {
            KeyMapping.set(w, true);   // what a real W press does: every mapping on W goes down
            if (walkTicks == 15) capture(mc, "phone_hotkey", false);
            return;
        }
        if (walkTicks == 31) {
            KeyMapping.set(w, false);
            return;
        }
        if (walkTicks < 45) return;
        double moved = mc.player.position().subtract(walkFrom).horizontalDistance();
        int sent = PhoneClient.hotkeysSent() - sentBefore;
        PhoneClient.SLOTS[0].setKey(InputConstants.UNKNOWN);
        KeyMapping.resetMapping();
        if (moved < 1.5) throw new IllegalStateException(String.format("W with hotkey slot 1 on it walked only %.2f blocks", moved));
        if (sent < 2) throw new IllegalStateException("hotkey slot 1 sent " + sent + " packets (want down and up)");
        EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS phone_hotkey moved={} hotkeys={} keyUpStillBound={}",
                String.format("%.2f", moved), sent, mc.options.keyUp.getKey().getName());
        mc.player.connection.sendCommand("ecmvisual phone dock");
        phase = 4;
        settled = 0;
    }

    /** A docked phone: the Charging Dock's model with its lamp lit and the phone standing in it, showing its charge bar. */
    private static void dock(Minecraft mc) throws Exception {
        net.minecraft.core.BlockPos pos = com.example.evanscomputermod.testing.PhoneServerChecks.ORIGIN.offset(2, 0, 0);
        var state = mc.level.getBlockState(pos);
        boolean ready = state.getBlock() instanceof com.example.evanscomputermod.phone.ChargingDockBlock
                && state.getValue(com.example.evanscomputermod.phone.ChargingDockBlock.ACTIVE)
                && mc.level.getBlockEntity(pos) instanceof com.example.evanscomputermod.phone.ChargingDockBlockEntity be
                && !be.phone().isEmpty() && mc.levelRenderer.hasRenderedAllSections();
        if (!ready) {
            settled = 0;
            return;
        }
        if (++settled < SETTLE) return;
        Path dir = Path.of(System.getProperty("ecm.visualOutput"));
        int bar = 0;
        try (NativeImage img = Screenshot.takeScreenshot(mc.getMainRenderTarget())) {
            img.writeToFile(dir.resolve("phone_dock.png"));
            int w = img.getWidth(), h = img.getHeight();
            // The charge bar (0x3FB950 above 50 %) in the middle of the view, where the dock is.
            for (int y = h / 4; y < h * 3 / 4; y++)
                for (int x = w / 4; x < w * 3 / 4; x++) {
                    int abgr = img.getPixelRGBA(x, y);
                    int r = abgr & 0xFF, g = (abgr >> 8) & 0xFF, b = (abgr >> 16) & 0xFF;
                    if (g > 110 && g - r > 50 && g - b > 20) bar++;
                }
        }
        if (bar < 10) throw new IllegalStateException("dock: no charge bar visible (" + bar + " green pixels)");
        EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS phone_dock bar={}", bar);
        mc.player.connection.sendCommand("ecmvisual phone finish");
        phase = 5;
        stopTicks = 40;
    }

    /**
     * Screenshot {@code name}.png and count the phone shell's teal wallpaper
     * pixels (0x0F3B4A at the top) in the half of the view the phone is in
     * (right for the main hand, left for the off hand), above the hotbar.
     */
    private static int capture(Minecraft mc, String name, boolean right) throws Exception {
        Path dir = Path.of(System.getProperty("ecm.visualOutput"));
        Files.createDirectories(dir);
        try (NativeImage img = Screenshot.takeScreenshot(mc.getMainRenderTarget())) {
            img.writeToFile(dir.resolve(name + ".png"));
            int w = img.getWidth(), h = img.getHeight(), green = 0;
            int x0 = right ? w / 2 : 0, x1 = right ? w : w / 2;
            for (int y = h / 4; y < h * 85 / 100; y++)
                for (int x = x0; x < x1; x++) {
                    int abgr = img.getPixelRGBA(x, y);
                    int r = abgr & 0xFF, g = (abgr >> 8) & 0xFF, b = (abgr >> 16) & 0xFF;
                    if (r < 40 && g >= 40 && g < 110 && b >= 50 && b < 120 && g - r > 25) green++;
                }
            return green;
        }
    }
}
//?}
