package com.example.evanscomputermod.testing.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.ScreenBlockEntity;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.testing.RadioVisualLayout;
import com.example.evanscomputermod.testing.ScreenServerChecks;
import java.nio.file.*;
import java.util.*;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Screenshot;
import net.neoforged.neoforge.client.event.ClientTickEvent;

/**
 * Screen render suite, hidden-client half: once the cluster's anchor shows {@code gfxtest}'s
 * full-screen colour grid, captures the view and checks that every block of the 4x4 cluster
 * shows the picture rather than the plain dark face (members drawn after the anchor used to
 * paint over it).
 */
public final class ScreenClientChecks {
  private static final int SETTLE_TICKS = 10;
  private static final long TIMEOUT_MS = 150_000;
  private static int settled, stopTicks = -1;
  private static boolean done;
  private static long started;

  private ScreenClientChecks() {}

  public static void tick(ClientTickEvent.Post event) {
    if (!RadioVisualLayout.active("screen")) return;
    var mc = Minecraft.getInstance();
    if (stopTicks >= 0) {
      if (--stopTicks == 0) mc.stop();
      return;
    }
    if (done || mc.level == null || mc.player == null) return;
    if (started == 0) started = System.currentTimeMillis();
    try {
      if (System.currentTimeMillis() - started > TIMEOUT_MS)
        throw new IllegalStateException("Screen render scenario exceeded " + TIMEOUT_MS / 1000 + "s");
      mc.options.hideGui = true;
      mc.options.pauseOnLostFocus = false;
      mc.options.bobView().set(false);
      boolean ready =
          mc.player.getEyePosition().distanceTo(ScreenServerChecks.eye()) < 0.3
              && mc.levelRenderer.hasRenderedAllSections()
              && showingGrid(mc);
      if (!ready) {
        settled = 0;
        return;
      }
      if (++settled < SETTLE_TICKS) return;
      capture(mc);
      done = true;
      mc.player.connection.sendCommand("ecmvisual screen finish");
      stopTicks = 20;
    } catch (Throwable t) {
      EvansComputerMod.LOGGER.error("ECM_VISUAL_CLIENT_FAIL", t);
      mc.stop();
    }
  }

  /** gfxtest's colour-grid phase: indexed pixels non-zero at a corner, the centre and bottom left. */
  private static boolean showingGrid(Minecraft mc) {
    if (!(mc.level.getBlockEntity(ScreenServerChecks.anchor()) instanceof ScreenBlockEntity s)
        || !s.isAnchor()
        || !s.isActive()) return false;
    TerminalDisplay d = s.clientDisplay;
    if (d == null || d.getPixelFormat() != TerminalDisplay.PIXEL_FORMAT_INDEXED8) return false;
    int w = d.getGfxWidth(), h = d.getGfxHeight();
    byte[] px = d.getPixelData();
    if (w <= 0 || h <= 0 || px == null || px.length < w * h) return false;
    return px[0] != 0 && px[(h / 2) * w + w / 2] != 0 && px[(h - 2) * w + 1] != 0;
  }

  private static void capture(Minecraft mc) throws Exception {
    var dir = Path.of(System.getProperty("ecm.visualOutput"));
    Files.createDirectories(dir);
    int n = ScreenServerChecks.SIZE;
    try (var image = Screenshot.takeScreenshot(mc.getMainRenderTarget())) {
      image.writeToFile(dir.resolve("screen_cluster.png"));
      int w = image.getWidth(), h = image.getHeight();
      double fov = Math.toRadians(mc.options.fov().get());
      double scale = (h / 2.0) / (ScreenServerChecks.DISTANCE * Math.tan(fov / 2));
      var report = new StringBuilder();
      var blank = new ArrayList<String>();
      for (int r = 0; r < n; r++) {
        for (int c = 0; c < n; c++) {
          // Tile centre relative to the cluster centre, in blocks (right, up).
          double dx = c + 0.5 - n / 2.0, dy = n / 2.0 - r - 0.5;
          // The grid has dark cells too, but a tile the picture covers is never almost all dark;
          // a painted-over tile is the face body colour (near black) throughout.
          int samples = 0, dark = 0;
          for (int i = -4; i <= 4; i++)
            for (int j = -4; j <= 4; j++) {
              int x = (int) Math.round(w / 2.0 + (dx + i * 0.08) * scale);
              int y = (int) Math.round(h / 2.0 - (dy + j * 0.08) * scale);
              if (x < 0 || y < 0 || x >= w || y >= h) continue;
              int abgr = image.getPixelRGBA(x, y);
              int max = Math.max(abgr & 0xFF, Math.max((abgr >> 8) & 0xFF, (abgr >> 16) & 0xFF));
              samples++;
              if (max < 40) dark++;
            }
          report.append(String.format("r%dc%d=%d/%d ", r, c, dark, samples));
          if (samples == 0 || dark * 4 >= samples * 3) blank.add("row " + r + " col " + c);
        }
      }
      EvansComputerMod.LOGGER.info("ECM_VISUAL_SCREEN_TILES {}", report.toString().trim());
      if (!blank.isEmpty())
        throw new IllegalStateException(
            "Screen blocks not showing the picture (row 0 is the top): " + blank);
      EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS screen_cluster tiles={}", n * n);
    }
  }
}
//?}
