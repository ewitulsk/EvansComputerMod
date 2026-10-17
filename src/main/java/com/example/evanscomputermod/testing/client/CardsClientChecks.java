package com.example.evanscomputermod.testing.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.card.client.CardItemRenderer;
import com.example.evanscomputermod.card.client.CardPrinterScreen;
import com.example.evanscomputermod.testing.CardsServerChecks;
import com.example.evanscomputermod.testing.RadioVisualLayout;
import com.mojang.blaze3d.platform.NativeImage;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Screenshot;
import net.minecraft.client.gui.screens.inventory.InventoryScreen;
import net.minecraft.core.Direction;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.client.event.ClientTickEvent;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashSet;

/**
 * Cards render suite, hidden-client half: photographs the three blocks (reader with a card,
 * printer, NFC Reader with its field on), a printed bank card in the hand (the renderer must
 * have drawn a bank card's faces, not the blank look), the four framed cards, the Card
 * Printer's screen (opened by right-clicking the block, as a player does) and the inventory
 * with the cards as icons. Each case is asked of the server first ({@code /ecmvisual cards
 * <case>}), which moves the player and checks the state.
 */
public final class CardsClientChecks {
    private static final long TIMEOUT_MS = 180_000;
    private static final String[] CASES = {"blocks", "hand", "frames", "printer", "inventory"};

    private static int phase, ticks, stopTicks = -1;
    private static long started;

    private CardsClientChecks() {}

    public static void tick(ClientTickEvent.Post event) {
        if (!RadioVisualLayout.active("cards")) return;
        Minecraft mc = Minecraft.getInstance();
        if (stopTicks >= 0) {
            if (--stopTicks == 0) mc.stop();
            return;
        }
        if (mc.level == null || mc.player == null) return;
        if (started == 0) started = System.currentTimeMillis();
        try {
            if (System.currentTimeMillis() - started > TIMEOUT_MS) throw new IllegalStateException("cards suite timed out in phase " + phase);
            mc.options.pauseOnLostFocus = false;
            mc.options.bobView().set(false);
            ticks++;
            if (phase >= CASES.length) {
                mc.player.connection.sendCommand("ecmvisual cards finish");
                stopTicks = 40;
                phase++;
                return;
            }
            String name = CASES[phase];
            if (ticks == 1) {
                // Let the fixture build (the first tick) and the player land; then ask for the case.
                return;
            }
            if (ticks == 40) {
                mc.player.connection.sendCommand("ecmvisual cards " + name);
                return;
            }
            if (ticks < 80) return;
            switch (name) {
                case "blocks" -> done(capture(mc, "cards_blocks"));
                case "hand" -> {
                    mc.options.hideGui = false;   // the HUD-less capture mode hides the hand too
                    String drew = CardItemRenderer.lastFirstPerson;
                    var held = mc.player.getMainHandItem();
                    if (ticks % 40 == 0) {
                        var model = mc.getItemRenderer().getModel(held, mc.level, mc.player, 0);
                        EvansComputerMod.LOGGER.info("ECM_VISUAL_CARDS_HAND tick={} held={} custom={} missing={} drew='{}' hideGui={} camera={}", ticks, held,
                                model.isCustomRenderer(), model == mc.getModelManager().getMissingModel(), drew, mc.options.hideGui, mc.options.getCameraType());
                    }
                    if (ticks < 100 || (drew.isEmpty() && ticks < 300)) return;
                    capture(mc, "cards_hand");
                    mc.options.hideGui = true;
                    if (!drew.startsWith("bank:") || drew.endsWith("blank"))
                        throw new IllegalStateException("the hand should show a printed bank card, the renderer drew '" + drew + "' (gui drew '" + CardItemRenderer.lastGui + "')");
                    EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS cards_hand drew={}", drew);
                    done(true);
                }
                case "frames" -> done(capture(mc, "cards_frames"));
                case "printer" -> {
                    if (ticks == 80) {
                        // Right-click the printer: its screen opens (nothing is loaded by hand).
                        var pos = CardsServerChecks.PRINTER;
                        var hit = new BlockHitResult(Vec3.atCenterOf(pos).add(-0.5, 0, 0), Direction.WEST, pos, false);
                        mc.gameMode.useItemOn(mc.player, InteractionHand.MAIN_HAND, hit);
                        return;
                    }
                    if (ticks < 110) return;
                    if (!(mc.screen instanceof CardPrinterScreen)) throw new IllegalStateException("the printer screen didn't open: " + mc.screen);
                    capture(mc, "cards_printer_screen");
                    mc.setScreen(null);
                    done(true);
                }
                case "inventory" -> {
                    if (ticks == 80) {
                        mc.setScreen(new InventoryScreen(mc.player));
                        return;
                    }
                    if (ticks < 100) return;
                    capture(mc, "cards_inventory");
                    mc.setScreen(null);
                    done(true);
                }
                default -> throw new IllegalStateException(name);
            }
        } catch (Throwable t) {
            EvansComputerMod.LOGGER.error("ECM_VISUAL_CLIENT_FAIL", t);
            mc.stop();
        }
    }

    private static void done(boolean ok) {
        phase++;
        ticks = 0;
    }

    private static boolean capture(Minecraft mc, String name) throws Exception {
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
        return true;
    }
}
//?}
