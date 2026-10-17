package com.example.evanscomputermod.testing.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.testing.RadioVisualLayout;
import com.mojang.blaze3d.platform.NativeImage;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Screenshot;
import net.minecraft.client.gui.screens.inventory.InventoryScreen;
import net.neoforged.neoforge.client.event.ClientTickEvent;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashSet;

/**
 * Disk render suite, hidden-client half: photographs the two Disk Drives (one empty, one
 * with a floppy in its slot) and the inventory with a floppy and a drive as icons. Each case
 * is asked of the server first ({@code /ecmvisual disks <case>}), which moves the player and
 * checks the state. The client checks the models it draws are the mod's, not the missing model.
 */
public final class DiskClientChecks {
    private static final long TIMEOUT_MS = 120_000;
    private static final String[] CASES = {"blocks", "inventory"};

    private static int phase, ticks, stopTicks = -1;
    private static long started;

    private DiskClientChecks() {}

    public static void tick(ClientTickEvent.Post event) {
        if (!RadioVisualLayout.active("disks")) return;
        Minecraft mc = Minecraft.getInstance();
        if (stopTicks >= 0) {
            if (--stopTicks == 0) mc.stop();
            return;
        }
        if (mc.level == null || mc.player == null) return;
        if (started == 0) started = System.currentTimeMillis();
        try {
            if (System.currentTimeMillis() - started > TIMEOUT_MS) throw new IllegalStateException("disks suite timed out in phase " + phase);
            mc.options.pauseOnLostFocus = false;
            mc.options.bobView().set(false);
            ticks++;
            if (phase >= CASES.length) {
                mc.player.connection.sendCommand("ecmvisual disks finish");
                stopTicks = 40;
                phase++;
                return;
            }
            String name = CASES[phase];
            if (ticks == 1) return;
            if (ticks == 40) {
                mc.player.connection.sendCommand("ecmvisual disks " + name);
                return;
            }
            if (ticks < 80) return;
            switch (name) {
                case "blocks" -> {
                    var missing = mc.getModelManager().getMissingModel();
                    var empty = mc.getBlockRenderer().getBlockModel(mc.level.getBlockState(com.example.evanscomputermod.testing.DiskServerChecks.EMPTY));
                    var full = mc.getBlockRenderer().getBlockModel(mc.level.getBlockState(com.example.evanscomputermod.testing.DiskServerChecks.FULL));
                    if (empty == missing || full == missing) throw new IllegalStateException("a drive draws the missing model");
                    if (empty == full) throw new IllegalStateException("the drive with a floppy draws the same model as the empty one");
                    capture(mc, "disks_blocks");
                    done();
                }
                case "inventory" -> {
                    if (ticks == 80) {
                        mc.setScreen(new InventoryScreen(mc.player));
                        return;
                    }
                    if (ticks < 100) return;
                    var missing = mc.getModelManager().getMissingModel();
                    var floppy = mc.getItemRenderer().getModel(mc.player.getInventory().getItem(0), mc.level, mc.player, 0);
                    if (floppy == missing) throw new IllegalStateException("the floppy draws the missing model");
                    capture(mc, "disks_inventory");
                    mc.setScreen(null);
                    done();
                }
                default -> throw new IllegalStateException(name);
            }
        } catch (Throwable t) {
            EvansComputerMod.LOGGER.error("ECM_VISUAL_CLIENT_FAIL", t);
            mc.stop();
        }
    }

    private static void done() {
        phase++;
        ticks = 0;
    }

    private static void capture(Minecraft mc, String name) throws Exception {
        Path dir = Path.of(System.getProperty("ecm.visualOutput"));
        Files.createDirectories(dir);
        try (NativeImage image = Screenshot.takeScreenshot(mc.getMainRenderTarget())) {
            var colors = new HashSet<Integer>();
            for (int y = 0; y < image.getHeight(); y += 12)
                for (int x = 0; x < image.getWidth(); x += 12) colors.add(image.getPixelRGBA(x, y));
            if (colors.size() < 24) throw new IllegalStateException("Blank framebuffer for " + name + ": " + colors.size() + " colors");
            image.writeToFile(dir.resolve(name + ".png"));
            EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS {} colors={}", name, colors.size());
        }
    }
}
//?}
