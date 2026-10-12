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
 * Phone render suite, hidden-client half: the phone in the main hand and then
 * the off hand must show its screen (the renderer drew the display texture,
 * and the photo has the shell's green text where the phone is), and with
 * hotkey slot 1 bound to W, holding W both walks (the player moves) and sends
 * the hotkey (the server reports the {@code phone_hotkey} events).
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
                case 2 -> walk(mc);
                default -> {
                }
            }
        } catch (Throwable t) {
            EvansComputerMod.LOGGER.error("ECM_VISUAL_CLIENT_FAIL", t);
            mc.stop();
        }
    }

    private static boolean showing(UUID id, String last) {
        PhoneClient.Status st = PhoneClient.status(id);
        var e = ClientDisplayCache.entry(id, 0);
        return st != null && st.state() == PhoneState.AWAKE && e.hasKeyframe
                && ScenarioRun.screen(e.display).contains("PHONE_RENDER_OK") && last.startsWith("display:");
    }

    private static void mainHand(Minecraft mc) throws Exception {
        UUID id = PhoneItemData.id(mc.player.getMainHandItem());
        if (id == null || !showing(id, PhoneItemRenderer.lastFirstPerson)) {
            settled = 0;
            return;
        }
        if (++settled < SETTLE) return;
        int green = capture(mc, "phone_main_hand", true);
        if (green < 40) throw new IllegalStateException("main hand: only " + green + " green text pixels where the phone is");
        EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS phone_main_hand green={} drew={}", green, PhoneItemRenderer.lastFirstPerson);
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
        int green = capture(mc, "phone_off_hand", false);
        if (green < 40) throw new IllegalStateException("off hand: only " + green + " green text pixels where the phone is");
        EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS phone_off_hand green={} drew={}", green, PhoneItemRenderer.lastOffHand);
        // Hotkey slot 1 on W, the walk-forward key: both must work.
        PhoneClient.SLOTS[0].setKey(InputConstants.getKey("key.keyboard.w"));
        KeyMapping.resetMapping();
        sentBefore = PhoneClient.hotkeysSent();
        walkFrom = mc.player.position();
        walkTicks = 0;
        phase = 2;
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
        mc.player.connection.sendCommand("ecmvisual phone finish");
        phase = 3;
        stopTicks = 40;
    }

    /**
     * Screenshot {@code name}.png and count the shell's green text pixels (the
     * default foreground, 0x55FF55) in the half of the view the phone is in
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
                    if (g > 150 && r < 140 && b < 140 && g - r > 60) green++;
                }
            return green;
        }
    }
}
//?}
