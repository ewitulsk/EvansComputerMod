package com.example.evanscomputermod.testing.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.testing.RadioVisualLayout;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Screenshot;
import net.minecraft.client.renderer.block.model.BakedQuad;
import net.minecraft.client.resources.model.BakedModel;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.util.RandomSource;
import net.minecraft.world.entity.decoration.ItemFrame;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.RenderShape;
import net.minecraft.world.phys.AABB;
import net.neoforged.neoforge.client.event.ClientTickEvent;
import net.neoforged.neoforge.client.model.data.ModelData;

/**
 * Radio render suite, hidden-client half: for each view of {@link RadioVisualLayout} it asks the
 * server to move the camera, waits for the paired block states and compiled chunk sections,
 * verifies baked block/item models (no missing model, no missing-texture sprite), captures a
 * screenshot and logs {@code ECM_VISUAL_CLIENT_PASS <case>}.
 */
public final class RadioClientChecks {
  private static final int SETTLE_TICKS = 40;
  private static final long TIMEOUT_MS = 150_000;

  private static RadioVisualLayout.Layout layout;
  private static int index, settled, sinceRequest, stopTicks = -1;
  private static boolean requested, verifiedItems;
  private static long started;

  private RadioClientChecks() {}

  public static void tick(ClientTickEvent.Post event) {
    if (!RadioVisualLayout.active("radio")) return;
    var mc = Minecraft.getInstance();
    if (stopTicks >= 0) {
      // Let the finish command reach the server before the client disconnects.
      if (--stopTicks == 0) mc.stop();
      return;
    }
    if (mc.level == null || mc.player == null) return;
    if (started == 0) started = System.currentTimeMillis();
    try {
      if (layout == null) layout = RadioVisualLayout.compute();
      if (System.currentTimeMillis() - started > TIMEOUT_MS)
        throw new IllegalStateException(
            "Radio render scenario exceeded " + TIMEOUT_MS / 1000 + "s at view " + index);
      mc.options.hideGui = true;
      mc.options.pauseOnLostFocus = false;
      if (index >= layout.views().size()) {
        handheldScreen(mc);
        return;
      }
      var view = layout.views().get(index);
      if (!requested) {
        // Wait for the server's fixture (every placed state paired on this client) first.
        if (!statesMatch(mc, layout.blocks())) return;
        mc.player.connection.sendCommand("ecmvisual radio view " + view.name());
        requested = true;
        settled = 0;
        sinceRequest = 0;
        return;
      }
      boolean arrived = mc.player.position().distanceTo(view.feet()) < 0.3;
      // The server refuses views until its fixture settled; ask again.
      if (!arrived && ++sinceRequest > 60) {
        requested = false;
        return;
      }
      boolean ready =
          arrived
              && statesMatch(mc, view.blocks())
              && sectionsCompiled(mc, view)
              && (!view.name().equals(RadioVisualLayout.ITEMS) || framesVisible(mc));
      if (!ready) {
        settled = 0;
        return;
      }
      if (++settled < SETTLE_TICKS) return;
      if (view.name().equals(RadioVisualLayout.ITEMS)) verifyItems(mc);
      else verifyBlocks(mc, view.blocks());
      capture(mc, view.name());
      index++;
      requested = false;
      if (index == layout.views().size()) handheldPhase = 1;
    } catch (Throwable t) {
      EvansComputerMod.LOGGER.error("ECM_VISUAL_CLIENT_FAIL", t);
      mc.stop();
    }
  }

  private static int handheldPhase, handheldTicks;

  /**
   * The handheld tuning screen: get a Handheld Radio, open its screen, type "11.6" in the
   * frequency box and press Enter (real key events into the screen), then the server checks
   * the item holds SW 11.6 MHz and the screen is captured.
   */
  private static void handheldScreen(Minecraft mc) throws Exception {
    handheldTicks++;
    switch (handheldPhase) {
      case 1 -> {
        mc.player.connection.sendCommand("ecmvisual radio handheld give");
        handheldPhase = 2;
        handheldTicks = 0;
      }
      case 2 -> {
        if (!(mc.player.getMainHandItem().getItem() instanceof com.example.evanscomputermod.radio.handheld.HandheldRadioItem)) {
          if (handheldTicks > 100) throw new IllegalStateException("no Handheld Radio arrived in the main hand");
          return;
        }
        com.example.evanscomputermod.radio.handheld.client.HandheldScreen.open(net.minecraft.world.InteractionHand.MAIN_HAND);
        handheldPhase = 3;
        handheldTicks = 0;
      }
      case 3 -> {
        if (handheldTicks < 5) return;
        var screen = mc.screen;
        if (!(screen instanceof com.example.evanscomputermod.radio.handheld.client.HandheldScreen))
          throw new IllegalStateException("the handheld screen didn't open: " + screen);
        for (char ch : "11.6".toCharArray()) screen.charTyped(ch, 0);
        screen.keyPressed(org.lwjgl.glfw.GLFW.GLFW_KEY_ENTER, 0, 0);
        handheldPhase = 4;
        handheldTicks = 0;
      }
      case 4 -> {
        if (handheldTicks < 20) return;   // settings packet reaches the server, the item syncs back
        mc.player.connection.sendCommand("ecmvisual radio handheld check");
        capture(mc, "radio_handheld_screen");
        handheldPhase = 5;
        handheldTicks = 0;
      }
      case 5 -> {
        if (handheldTicks < 10) return;
        mc.setScreen(null);
        mc.player.connection.sendCommand("ecmvisual radio finish");
        stopTicks = 20;
        handheldPhase = 6;
      }
      default -> {}
    }
  }

  private static boolean statesMatch(Minecraft mc, List<RadioVisualLayout.Placed> blocks) {
    for (var placed : blocks) if (!placed.matches(mc.level.getBlockState(placed.pos()))) return false;
    return true;
  }

  private static boolean sectionsCompiled(Minecraft mc, RadioVisualLayout.View view) {
    if (!mc.levelRenderer.hasRenderedAllSections()) return false;
    for (var placed : view.blocks())
      if (!mc.levelRenderer.isSectionCompiled(placed.pos())) return false;
    return true;
  }

  private static boolean framesVisible(Minecraft mc) {
    for (int i = 0; i < layout.items().size(); i++) {
      var item = BuiltInRegistries.ITEM.get(layout.items().get(i));
      var frames = mc.level.getEntitiesOfClass(ItemFrame.class, new AABB(layout.framePos(i)));
      if (frames.stream().noneMatch(f -> f.getItem().is(item))) return false;
    }
    return true;
  }

  private static boolean missing(BakedQuad quad) {
    return quad.getSprite().contents().name().getPath().contains("missingno");
  }

  private static void verifyBlocks(Minecraft mc, List<RadioVisualLayout.Placed> blocks) {
    var missingModel = mc.getModelManager().getMissingModel();
    var problems = new ArrayList<String>();
    for (var placed : blocks) {
      var state = mc.level.getBlockState(placed.pos());
      BakedModel model = mc.getBlockRenderer().getBlockModel(state);
      if (model == missingModel) {
        problems.add(placed.label() + ": missing block model");
        continue;
      }
      if (model.getParticleIcon(ModelData.EMPTY).contents().name().getPath().contains("missingno"))
        problems.add(placed.label() + ": missing particle texture");
      var quads = new ArrayList<BakedQuad>();
      var sides = new ArrayList<Direction>(Arrays.asList(Direction.values()));
      sides.add(null);
      var data = mc.level.getModelData(placed.pos());
      for (var side : sides)
        for (var type : model.getRenderTypes(state, RandomSource.create(42), data))
          quads.addAll(model.getQuads(state, side, RandomSource.create(42), data, type));
      if (state.getRenderShape() == RenderShape.MODEL && quads.isEmpty())
        problems.add(placed.label() + ": baked model has no quads");
      var sprites = new TreeSet<String>();
      for (var q : quads) sprites.add(q.getSprite().contents().name().toString());
      if (quads.stream().anyMatch(RadioClientChecks::missing))
        problems.add(placed.label() + ": quads use the missing texture");
      EvansComputerMod.LOGGER.info(
          "ECM_VISUAL_RADIO_MODEL {} quads={} renderShape={} sprites={}",
          placed.label(),
          quads.size(),
          state.getRenderShape(),
          sprites);
    }
    if (!problems.isEmpty())
      throw new IllegalStateException("Radio block models broken: " + String.join("; ", problems));
  }

  private static void verifyItems(Minecraft mc) {
    var missingModel = mc.getModelManager().getMissingModel();
    var problems = new ArrayList<String>();
    for (var id : layout.items()) {
      var stack = new ItemStack(BuiltInRegistries.ITEM.get(id));
      BakedModel model = mc.getItemRenderer().getModel(stack, mc.level, mc.player, 0);
      if (model == missingModel) {
        problems.add(id + ": missing item model");
        continue;
      }
      var quads = new ArrayList<BakedQuad>();
      for (var side : Direction.values()) quads.addAll(model.getQuads(null, side, RandomSource.create(42)));
      quads.addAll(model.getQuads(null, null, RandomSource.create(42)));
      boolean particleMissing =
          model.getParticleIcon().contents().name().getPath().contains("missingno");
      if (particleMissing) problems.add(id + ": missing particle texture");
      if (quads.stream().anyMatch(RadioClientChecks::missing))
        problems.add(id + ": quads use the missing texture");
      if (quads.isEmpty() && !model.isCustomRenderer()) problems.add(id + ": item model has no quads");
      var sprites = new TreeSet<String>();
      for (var q : quads) sprites.add(q.getSprite().contents().name().toString());
      EvansComputerMod.LOGGER.info(
          "ECM_VISUAL_RADIO_ITEM_MODEL {} quads={} gui3d={} sprites={}",
          id,
          quads.size(),
          model.isGui3d(),
          sprites);
    }
    if (!problems.isEmpty())
      throw new IllegalStateException("Radio item models broken: " + String.join("; ", problems));
  }

  private static void capture(Minecraft mc, String name) throws Exception {
    var dir = Path.of(System.getProperty("ecm.visualOutput"));
    Files.createDirectories(dir);
    try (var image = Screenshot.takeScreenshot(mc.getMainRenderTarget())) {
      var colors = new HashSet<Integer>();
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
