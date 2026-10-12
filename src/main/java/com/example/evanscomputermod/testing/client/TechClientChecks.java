package com.example.evanscomputermod.testing.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.*;
import com.example.evanscomputermod.testing.TechServerChecks;
import java.nio.file.*;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Screenshot;
import net.minecraft.util.RandomSource;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.client.event.ClientChatReceivedEvent;
import net.neoforged.neoforge.client.event.ClientTickEvent;

/**
 * Tech Village suite, hidden real client: the server announces each view with an
 * {@code ECMSHOT <name> <x> <y> <z>} message after moving the player; the client waits for
 * the arrival and compiled chunk sections, checks the baked fiber models, captures a
 * nonblank screenshot and answers {@code ecmvisual shotdone <name>}. For the store phone the
 * server sends {@code ECMPHONE open|tap X Y|shot NAME|close}: the client opens the held phone
 * full screen (as the use key does), taps its screen with the mouse at phone pixels, and
 * photographs it.
 */
public final class TechClientChecks {
  private static final int SETTLE_TICKS = 40, MAX_WAIT_TICKS = 600;
  private static final long TIMEOUT_MS = 900_000;
  private static int settled, waited, connectedQuads, stopTicks = -1;
  private static long started;
  private static volatile String shot;
  private static volatile Vec3 shotFeet;
  private static volatile boolean end;
  private static boolean verified;

  private static final java.util.concurrent.ConcurrentLinkedQueue<String> phoneCommands = new java.util.concurrent.ConcurrentLinkedQueue<>();
  private static String phoneShotName;
  private static int phoneTicks;

  public static void chat(ClientChatReceivedEvent event) {
    if (!Boolean.getBoolean("ecm.clientChecks")) return;
    String text = event.getMessage().getString();
    if (text.startsWith("ECMPHONE ")) {
      phoneCommands.add(text.substring("ECMPHONE ".length()));
      return;
    }
    if (text.equals("ECMSHOT_END")) end = true;
    else if (text.startsWith("ECMSHOT ")) {
      String[] parts = text.split(" ");
      shotFeet = new Vec3(Double.parseDouble(parts[2]), Double.parseDouble(parts[3]), Double.parseDouble(parts[4]));
      shot = parts[1];
      settled = 0;
      waited = 0;
    }
  }

  public static void tick(ClientTickEvent.Post event) {
    if (!Boolean.getBoolean("ecm.clientChecks")) return;
    var mc = Minecraft.getInstance();
    if (stopTicks >= 0) {
      if (--stopTicks == 0) mc.stop();
      return;
    }
    if (mc.level == null || mc.player == null) {
      // The server ended the session (finished or failed): do not idle until the timeout.
      if (started != 0) {
        EvansComputerMod.LOGGER.info("Tech client: disconnected, stopping");
        mc.stop();
      }
      return;
    }
    if (started == 0) started = System.currentTimeMillis();
    try {
      if (System.currentTimeMillis() - started > TIMEOUT_MS)
        throw new IllegalStateException("Tech client scenario exceeded " + TIMEOUT_MS / 1000 + " s at " + shot);
      mc.options.hideGui = true;
      mc.options.pauseOnLostFocus = false;
      if (end && shot == null) {
        stopTicks = 20;
        return;
      }
      if (storePhone(mc)) return;
      String name = shot;
      if (name == null) return;
      boolean arrived = mc.player.position().distanceTo(shotFeet) < 1.0;
      // A screen (the chat terminal) renders over the world: section compilation is irrelevant.
      boolean compiled = mc.levelRenderer.hasRenderedAllSections() || mc.screen != null;
      var middle = mc.level.getBlockState(TechServerChecks.FIXTURE);
      boolean stateReady =
          switch (name) {
            case "fiber_connected", "fiber_repaired" ->
                middle.is(ModBlocks.FIBER_SPAN.get()) && middle.getValue(NetworkCableBlock.EAST);
            case "fiber_disconnected" ->
                middle.is(ModBlocks.FIBER_SPAN.get()) && !middle.getValue(NetworkCableBlock.EAST);
            // The server opened the house PC's terminal for this view.
            case "chat_session" -> mc.screen instanceof TerminalScreen;
            default -> true;
          };
      // Far views may never report every section compiled; after the cap, capture anyway.
      if (!(arrived && stateReady && (compiled || ++waited > MAX_WAIT_TICKS))) {
        settled = 0;
        return;
      }
      if (++settled < SETTLE_TICKS) return;
      if (!verified) {
        verifyModels(mc);
        verified = true;
      }
      if (name.startsWith("fiber_") && stateReady && !name.equals("fiber_mast") && !name.equals("fiber_terrain")
          && !name.equals("fiber_valley")) {
        int quads = mc.getBlockRenderer().getBlockModel(middle).getQuads(middle, null, RandomSource.create(0)).size();
        if (name.equals("fiber_connected")) connectedQuads = quads;
        else if (name.equals("fiber_disconnected") && quads != connectedQuads - 6)
          throw new IllegalStateException("Disconnected model did not remove its six east-arm faces");
        else if (name.equals("fiber_repaired") && quads != connectedQuads)
          throw new IllegalStateException("Repaired model did not restore its east arm");
      }
      capture(mc, name);
      shot = null;
      mc.player.connection.sendCommand("ecmvisual shotdone " + name);
    } catch (Throwable t) {
      EvansComputerMod.LOGGER.error("ECM_VISUAL_CLIENT_FAIL", t);
      mc.stop();
    }
  }

  /** The store phone (server phase 17): open it, tap, photograph. True while it is busy. */
  private static boolean storePhone(Minecraft mc) throws Exception {
    if (phoneShotName != null) {
      if (!(mc.screen instanceof com.example.evanscomputermod.phone.client.PhoneScreen))
        throw new IllegalStateException("the phone closed before " + phoneShotName);
      if (++phoneTicks < 40) return true;   // the phone's frames reach the client
      capture(mc, phoneShotName);
      mc.player.connection.sendCommand("ecmvisual shotdone " + phoneShotName);
      phoneShotName = null;
      return true;
    }
    String cmd = phoneCommands.poll();
    if (cmd == null) return mc.screen instanceof com.example.evanscomputermod.phone.client.PhoneScreen;
    String[] w = cmd.split(" ");
    switch (w[0]) {
      case "open" -> {
        com.example.evanscomputermod.phone.client.PhoneScreen.open(net.minecraft.world.InteractionHand.MAIN_HAND);
        if (!(mc.screen instanceof com.example.evanscomputermod.phone.client.PhoneScreen)) throw new IllegalStateException("the phone didn't open");
        mc.player.connection.sendCommand("ecmvisual storephone opened");
      }
      case "tap" -> {
        if (!(mc.screen instanceof com.example.evanscomputermod.phone.client.PhoneScreen ps)) throw new IllegalStateException("tap: the phone isn't open");
        double[] at = ps.screenPoint(Integer.parseInt(w[1]), Integer.parseInt(w[2]));
        ps.mouseMoved(at[0], at[1]);
        ps.mouseClicked(at[0], at[1], 0);
        ps.mouseReleased(at[0], at[1], 0);
      }
      case "shot" -> {
        phoneShotName = w[1];
        phoneTicks = 0;
      }
      case "close" -> mc.setScreen(null);
      default -> throw new IllegalStateException("unknown ECMPHONE " + cmd);
    }
    return true;
  }

  private static void verifyModels(Minecraft mc) {
    var module =
        mc.getItemRenderer()
            .getModel(
                com.example.evanscomputermod.item.ModItems.ALWAYS_ON_MODULE.get().getDefaultInstance(),
                mc.level,
                mc.player,
                0);
    if (module.getParticleIcon().contents().name().getPath().contains("missing"))
      throw new IllegalStateException("Missing Always-On Module texture");
    for (var block :
        new net.minecraft.world.level.block.Block[] {
          ModBlocks.FIBER_SPAN.get(), ModBlocks.FIBER_PATCH_PANEL.get()
        }) {
      var model = mc.getBlockRenderer().getBlockModel(block.defaultBlockState());
      var quads = model.getQuads(block.defaultBlockState(), null, RandomSource.create(0));
      if (quads.isEmpty()
          || quads.stream().anyMatch(q -> q.getSprite().contents().name().getPath().contains("missing")))
        throw new IllegalStateException("Missing baked geometry or texture for " + block);
    }
  }

  private static void capture(Minecraft mc, String name) throws Exception {
    var dir = Path.of(System.getProperty("ecm.visualOutput"));
    Files.createDirectories(dir);
    try (var image = Screenshot.takeScreenshot(mc.getMainRenderTarget())) {
      var colors = new java.util.HashSet<Integer>();
      for (int y = 0; y < image.getHeight(); y += 12)
        for (int x = 0; x < image.getWidth(); x += 12) colors.add(image.getPixelRGBA(x, y));
      if (colors.size() < 24)
        throw new IllegalStateException("Blank framebuffer for " + name + ": " + colors.size() + " colors");
      image.writeToFile(dir.resolve(name + ".png"));
      EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS {} colors={}", name, colors.size());
    }
  }
}
//?}
