package com.example.evanscomputermod.testing.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.TerminalScreen;
import com.example.evanscomputermod.testing.RadioVisualLayout;
import com.example.evanscomputermod.testing.UploadServerChecks;
import com.example.evanscomputermod.testing.scenario.ScenarioRun;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Screenshot;
import net.neoforged.neoforge.client.event.ClientTickEvent;

/**
 * File upload suite, hidden-client half (see {@link UploadServerChecks}). In the Terminal's
 * GUI the server opened, types {@code upload} and Enter through the screen's own key
 * handlers. The upload sidecar in this game then "picks" a file: the system property
 * {@code evanscomputermod.upload.pick} stands in for the system file dialog (nobody can click
 * one in a hidden client), and everything after it is the real path: the sidecar reads the
 * file, sends it in chunks to the server, and the server's sidecar writes it into the
 * computer. Checks that the client's own copy of the screen shows {@code upload: saved} (case
 * {@code upload_terminal}); then that a cancelled pick ({@code -}) shows
 * {@code upload: cancelled} ({@code upload_cancelled}).
 */
public final class UploadClientChecks {
  private static final String PICK = "evanscomputermod.upload.pick";
  private static final long TIMEOUT_MS = 180_000;
  private static int phase, settle, stopTicks = -1;
  private static long started;

  private UploadClientChecks() {}

  public static void tick(ClientTickEvent.Post event) {
    if (!RadioVisualLayout.active("upload")) return;
    var mc = Minecraft.getInstance();
    if (stopTicks >= 0) {
      if (--stopTicks == 0) mc.stop();
      return;
    }
    if (phase >= 3 || mc.level == null || mc.player == null) return;
    if (started == 0) started = System.currentTimeMillis();
    try {
      if (System.currentTimeMillis() - started > TIMEOUT_MS)
        throw new IllegalStateException("upload suite exceeded " + TIMEOUT_MS / 1000 + "s (phase " + phase + ")");
      mc.options.pauseOnLostFocus = false;
      if (!(mc.screen instanceof TerminalScreen screen)) {
        settle = 0;
        return;
      }
      String text = ScenarioRun.screen(screen.getMenu().getBlockEntity().getDisplay());
      if (phase == 0) {
        // The shell is up (the server typed mkdir -p /home): let it settle, then upload.
        if (!text.contains("Welcome to Terminal OS") || ++settle < 40) return;
        Path file = mc.gameDirectory.toPath().resolve("upload-pick").resolve("postcard.txt");
        Files.createDirectories(file.getParent());
        Files.writeString(file, UploadServerChecks.CONTENT, StandardCharsets.UTF_8);
        System.setProperty(PICK, file.toAbsolutePath().toString());
        type(screen, "upload");
        phase = 1;
        settle = 0;
      } else if (phase == 1) {
        if (!text.contains("upload: saved /home/postcard.txt") || ++settle < 10) return;
        shot(mc, "upload_terminal");
        EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS upload_terminal");
        // Control: the player closes the picker without a file.
        System.setProperty(PICK, "-");
        type(screen, "upload");
        phase = 2;
        settle = 0;
      } else if (phase == 2) {
        if (!text.contains("upload: cancelled") || ++settle < 10) return;
        shot(mc, "upload_cancelled");
        EvansComputerMod.LOGGER.info("ECM_VISUAL_CLIENT_PASS upload_cancelled");
        mc.player.connection.sendCommand("ecmvisual upload finish");
        phase = 3;
        stopTicks = 40;
      }
    } catch (Throwable t) {
      EvansComputerMod.LOGGER.error("ECM_VISUAL_CLIENT_FAIL", t);
      mc.stop();
    }
  }

  /** Type a line into the Terminal GUI the way the keyboard does: characters, then Enter. */
  private static void type(TerminalScreen screen, String line) {
    for (char c : line.toCharArray()) screen.charTyped(c, 0);
    screen.keyPressed(org.lwjgl.glfw.GLFW.GLFW_KEY_ENTER, 0, 0);
  }

  private static void shot(Minecraft mc, String name) throws Exception {
    var dir = Path.of(System.getProperty("ecm.visualOutput"));
    Files.createDirectories(dir);
    try (var image = Screenshot.takeScreenshot(mc.getMainRenderTarget())) {
      image.writeToFile(dir.resolve(name + ".png"));
    }
  }
}
//?}
