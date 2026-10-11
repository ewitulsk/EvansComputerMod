package com.example.evanscomputermod.testing.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.*;
import java.nio.file.*;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Screenshot;
import net.minecraft.core.BlockPos;
import net.minecraft.util.RandomSource;
import net.neoforged.neoforge.client.event.ClientTickEvent;

/** Hidden real Minecraft client: verifies baked models and captures paired server states. */
public final class TechClientChecks {
  private static int stage, settled, connectedQuads;
  private static long started;
  private static final BlockPos MIDDLE = new BlockPos(3, 108, 0);

  public static void tick(ClientTickEvent.Post event) {
    if (!Boolean.getBoolean("ecm.clientChecks")) return;
    var mc = Minecraft.getInstance();
    if (mc.level == null || mc.player == null) return;
    if (started == 0) started = System.currentTimeMillis();
    try {
      if (System.currentTimeMillis() - started > 55_000)
        throw new IllegalStateException(
            "Client render scenario exceeded 55 seconds at stage " + stage);
      mc.options.hideGui = true;
      mc.options.pauseOnLostFocus = false;
      var state = mc.level.getBlockState(MIDDLE);
      boolean ready =
          switch (stage) {
            case 0 ->
                state.is(ModBlocks.FIBER_SPAN.get())
                    && state.getValue(NetworkCableBlock.EAST)
                    && mc.player.getZ() > 8
                    && mc.player.getY() > 100;
            case 1 ->
                state.is(ModBlocks.FIBER_SPAN.get()) && !state.getValue(NetworkCableBlock.EAST);
            case 2 ->
                state.is(ModBlocks.FIBER_SPAN.get()) && state.getValue(NetworkCableBlock.EAST);
            case 3 -> Math.abs(mc.player.getX()) > 1000 || Math.abs(mc.player.getZ()) > 1000;
            default -> false;
          };
      if (!ready) {
        settled = 0;
        return;
      }
      if (++settled < 35) return;
      settled = 0;
      switch (stage++) {
        case 0 -> {
          verifyModels(mc);
          connectedQuads =
              mc.getBlockRenderer()
                  .getBlockModel(state)
                  .getQuads(state, null, RandomSource.create(0))
                  .size();
          capture(mc, "fiber_connected");
          mc.player.connection.sendCommand("ecmvisual disconnect");
        }
        case 1 -> {
          if (mc.getBlockRenderer()
                  .getBlockModel(state)
                  .getQuads(state, null, RandomSource.create(0))
                  .size()
              != connectedQuads - 6)
            throw new IllegalStateException(
                "Disconnected model did not remove its six east-arm faces");
          capture(mc, "fiber_disconnected");
          mc.player.connection.sendCommand("ecmvisual repair");
        }
        case 2 -> {
          if (mc.getBlockRenderer()
                  .getBlockModel(state)
                  .getQuads(state, null, RandomSource.create(0))
                  .size()
              != connectedQuads)
            throw new IllegalStateException("Repaired model did not restore its east arm");
          capture(mc, "fiber_repaired");
          mc.player.connection.sendCommand("ecm techvillage tp 3");
        }
        case 3 -> {
          mc.player.setXRot(0);
          capture(mc, "village_arrival");
          mc.player.connection.sendCommand("ecmvisual finish");
        }
        default -> {}
      }
      if (stage == 4) mc.stop();
    } catch (Throwable t) {
      EvansComputerMod.LOGGER.error("ECM_VISUAL_CLIENT_FAIL", t);
      mc.stop();
    }
  }

  private static void verifyModels(Minecraft mc) {
    var module =
        mc.getItemRenderer()
            .getModel(
                com.example.evanscomputermod.item.ModItems.ALWAYS_ON_MODULE
                    .get()
                    .getDefaultInstance(),
                mc.level,
                mc.player,
                0);
    if (module.getParticleIcon().contents().name().getPath().contains("missing"))
      throw new IllegalStateException("Missing Always-On Module texture");
    for (var block :
        new net.minecraft.world.level.block.Block[] {
          ModBlocks.UTILITY_POLE.get(),
          ModBlocks.FIBER_SPAN.get(),
          ModBlocks.FIBER_PATCH_PANEL.get()
        }) {
      var model = mc.getBlockRenderer().getBlockModel(block.defaultBlockState());
      var quads = model.getQuads(block.defaultBlockState(), null, RandomSource.create(0));
      if (quads.isEmpty()
          || quads.stream()
              .anyMatch(q -> q.getSprite().contents().name().getPath().contains("missing")))
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
        throw new IllegalStateException(
            "Blank framebuffer for " + name + ": " + colors.size() + " colors");
      image.writeToFile(dir.resolve(name + ".png"));
      EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS {} colors={}", name, colors.size());
    }
  }
}
//?}
