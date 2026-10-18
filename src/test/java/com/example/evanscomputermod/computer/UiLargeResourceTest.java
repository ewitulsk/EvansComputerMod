package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.api.IFramebufferDisplay;
import com.example.evanscomputermod.api.IRedstoneProvider;
import com.example.evanscomputermod.api.IWorldAccess;
import com.example.evanscomputermod.computer.ui.ClientUiScenes;
import com.example.evanscomputermod.computer.ui.Compositor;
import com.example.evanscomputermod.computer.ui.UiClients;
import com.example.evanscomputermod.computer.ui.UiSceneCodec;
import com.example.evanscomputermod.computer.ui.UiManager;
import com.example.evanscomputermod.wasm.WasmManager;
import com.example.evanscomputermod.wasm.chicory.ChicoryRuntimeProvider;
import net.minecraft.server.MinecraftServer;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.UUID;
import java.util.function.BooleanSupplier;
import java.util.function.Predicate;

import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * A large image resource on the real kernel (Chicory, no world): an app that
 * uploads an n x n RGBA image ({@code apptest ui-image n}) must keep answering
 * input for several seconds after its window shows. A 160 x 160 image (the
 * Music app's cover art, 100 KiB) used to freeze the window a few seconds in.
 */
public class UiLargeResourceTest {
    private static final String[] PROGRAMS = {"gui", "desktop", "apptest"};
    private ComputerInstance computer;
    private TerminalDisplay display;
    private Path binDir;
    private Path dataDir;

    private static Path repoRoot() {
        Path p = Path.of("").toAbsolutePath();
        while (p != null && !Files.exists(p.resolve("rust/Cargo.toml"))) p = p.getParent();
        return p;
    }

    @BeforeEach
    void boot() throws Exception {
        Path root = repoRoot();
        assumeTrue(root != null, "repository root not found");
        Path kernel = root.resolve("rust/target/wasm32-unknown-unknown/release/terminal_os.wasm");
        Path progs = root.resolve("rust/target/wasm32-wasip1/release");
        assumeTrue(Files.exists(kernel), "kernel not built: " + kernel);
        for (String p : PROGRAMS) assumeTrue(Files.exists(progs.resolve(p + ".wasm")), p + ".wasm not built in " + progs);
        binDir = Files.createTempDirectory("ecm-ui-bigres-bin");
        Files.copy(kernel, binDir.resolve("terminal_os.wasm"));
        for (String p : PROGRAMS) Files.copy(progs.resolve(p + ".wasm"), binDir.resolve(p + ".wasm"));
        WasmManager.setWasmBinPathForTesting(binDir);
        WasmManager.bind(new ChicoryRuntimeProvider().create());

        display = new TerminalDisplay();
        UUID id = UUID.randomUUID();
        dataDir = Path.of("computer-data", id.toString());
        Files.createDirectories(dataDir.resolve("etc"));
        Files.writeString(dataDir.resolve("etc/ui.toml"), "autostart = true\n");
        // apptest installed as an app too (launched through the app manager, like Music).
        Path app = dataDir.resolve("apps/dev.test.image");
        Files.createDirectories(app);
        Files.writeString(app.resolve("app.toml"), "id = \"dev.test.image\"\nname = \"Image Test\"\nversion = \"1.0.0\"\n"
                + "entry = \"main.wasm\"\n");
        Files.copy(progs.resolve("apptest.wasm"), app.resolve("main.wasm"));
        computer = new ComputerInstance(new Host(id, display), new byte[0][]);
        computer.loadModule("terminal_os");
        computer.executeMain();
        computer.startWorkerThread();
        waitFor(() -> comp() != null && comp().shellConnected(), 30_000, "the desktop shell to connect");
        waitFor(() -> comp().layerTexts("taskbar").contains("Apps"), 10_000, "the taskbar");
    }

    @AfterEach
    void shutdown() throws Exception {
        viewing = false;
        if (viewer != null) viewer.join(2_000);
        if (computer != null) computer.close();
        if (binDir != null) deleteTree(binDir);
        if (dataDir != null) deleteTree(dataDir);
    }

    private UiManager ui() {
        return computer.getUiManager();
    }

    private Compositor comp() {
        return ui().compositor();
    }

    private void click(int[] at) {
        computer.handlePointerInput(IComputerHost.DISPLAY_OWN, (byte) 1, at[0], at[1], (byte) 0, (byte) 0, (byte) 0);
        computer.handlePointerInput(IComputerHost.DISPLAY_OWN, (byte) 2, at[0], at[1], (byte) 1, (byte) 0, (byte) 0);
        computer.handlePointerInput(IComputerHost.DISPLAY_OWN, (byte) 3, at[0], at[1], (byte) 0, (byte) 0, (byte) 0);
    }

    @Test
    void smallImageAppKeepsAnsweringInput() throws Exception {
        clicksAnswered(64, "");
    }

    @Test
    void largeImageAppKeepsAnsweringInput() throws Exception {
        clicksAnswered(160, "");
    }

    @Test
    void largeImageUploadedWhileShownKeepsAnsweringInput() throws Exception {
        clicksAnswered(160, " late");
    }

    @Test
    void largeImageOnARedrawnPageKeepsAnsweringInput() throws Exception {
        clicksAnswered(160, " page");
    }

    @Test
    void largeImageInAnInstalledAppKeepsAnsweringInput() throws Exception {
        clicksAnswered(160, " page", "dev.test.image");
    }

    /**
     * A player whose client draws the GUI itself (the real hub and
     * {@link ClientUiScenes}), pumped like the server's display sync.
     */
    private final UUID player = UUID.randomUUID();
    private volatile boolean viewing;
    private Thread viewer;

    private void startViewer() {
        UiClients.hello(player, UiSceneCodec.VERSION, ClientUiScenes.heldImageKeys());
        viewing = true;
        viewer = new Thread(() -> {
            while (viewing) {
                try {
                    pumpViewer();
                    Thread.sleep(50);
                } catch (InterruptedException e) {
                    return;
                }
            }
        }, "scene-viewer");
        viewer.setDaemon(true);
        viewer.start();
    }

    /** One display sync pass to the viewer and its client's handling of it (acks included), then it draws. */
    private synchronized void pumpViewer() {
        UUID cid = computer.getHost().getComputerId();
        for (int round = 0; round < 8; round++) {
            List<byte[]> sent = new ArrayList<>();
            ui().sceneSync(9, List.of(player), (p, d, data) -> sent.add(data));
            if (sent.isEmpty()) break;
            for (byte[] data : sent) {
                ClientUiScenes.Ack ack = ClientUiScenes.onMessage(cid, 9, data, System.nanoTime() / 1_000_000L);
                if (ack != null) ui().sceneHub().onAck(player, 9, ack.epoch(), ack.generation(), ack.missing());
            }
        }
        ClientUiScenes.tick(System.nanoTime() / 1_000_000L);
    }

    /** Null if the viewer's client shows what the compositor shows, else how they differ (compositor held still). */
    private String clientDiffers() {
        Compositor c = comp();
        synchronized (c) {
            pumpViewer();
            byte[] server = c.frameRgba();
            TerminalDisplay d = com.example.evanscomputermod.network.ClientDisplayCache.entry(computer.getHost().getComputerId(), 9).display;
            synchronized (d) {
                byte[] px = d.getPixelData();
                if (px == null || px.length != server.length) return "the client display is " + d.getGfxWidth() + "x" + d.getGfxHeight();
                int diff = 0;
                for (int i = 0; i < server.length; i += 4) {
                    if (server[i] != px[i] || server[i + 1] != px[i + 1] || server[i + 2] != px[i + 2]) diff++;
                }
                return diff == 0 ? null : diff + " pixels differ";
            }
        }
    }

    private void clicksAnswered(int n, String mode) throws Exception {
        clicksAnswered(n, mode, "apptest");
    }

    private void clicksAnswered(int n, String mode, String prog) throws Exception {
        startViewer();
        computer.sendInput("gui launch " + prog + " ui-image " + n + mode + "\n");
        waitFor(() -> comp().findText("Image Test", "Clicks 0") != null, 30_000, "the Image Test window");
        // Several seconds of clicks: every one must reach the app and come back as a redraw.
        long end = System.currentTimeMillis() + 6_000;
        int clicks = 0;
        while (System.currentTimeMillis() < end) {
            String label = "Clicks " + clicks;
            int[] at = comp().findText("Image Test", label);
            assertNotNull(at, "the button shows '" + label + "' (" + n + "x" + n + " image); texts: " + texts());
            click(at);
            clicks++;
            String next = "Clicks " + clicks;
            waitFor(() -> comp().findText("Image Test", next) != null, 3_000,
                    "click " + clicks + " to be answered with a " + n + "x" + n + " image; texts: " + texts()
                            + "; GUI: " + ui().status());
            Thread.sleep(250);
        }
        assertTrue(clicks >= 10, "only " + clicks + " clicks in 6 s");
        // ...and the player's client, which draws the window itself, shows the same picture.
        viewing = false;
        viewer.join(2_000);
        String why = clientDiffers();
        for (long until = System.currentTimeMillis() + 3_000; why != null && System.currentTimeMillis() < until; why = clientDiffers()) {
            Thread.sleep(50);
        }
        assertNull(why, "the client shows the window as the compositor does (" + n + "x" + n + " image)");
        var v = ui().sceneHub().viewer(player, 9);
        System.out.println("ui-image " + n + mode + ": " + clicks + " clicks answered; scene viewer "
                + (v == null ? "none" : v.bytes() + " B, " + v.keyframes() + " keyframes, " + v.steps() + " steps, " + v.resources() + " images"));
    }

    private String texts() {
        for (Compositor.WindowInfo w : comp().windows()) {
            if (w.title().equals("Image Test")) return comp().textsOf(w.id()).toString();
        }
        return "(no window) " + comp().windows();
    }

    // ------------------------------------------------------------ helpers

    private static void waitFor(BooleanSupplier cond, long timeoutMs, String what) throws InterruptedException {
        long deadline = System.currentTimeMillis() + timeoutMs;
        while (System.currentTimeMillis() < deadline) {
            if (cond.getAsBoolean()) return;
            Thread.sleep(20);
        }
        assertTrue(cond.getAsBoolean(), "timed out waiting for " + what);
    }

    private String screen() {
        StringBuilder sb = new StringBuilder();
        for (int y = 0; y < display.getHeight(); y++) {
            for (int x = 0; x < display.getWidth(); x++) {
                int c = display.getCharAt(x, y) & 0xFF;
                sb.append(c >= 32 && c < 127 ? (char) c : ' ');
            }
            sb.append('\n');
        }
        return sb.toString();
    }

    private void waitForScreen(Predicate<String> cond, long timeoutMs, String what) throws InterruptedException {
        long deadline = System.currentTimeMillis() + timeoutMs;
        while (System.currentTimeMillis() < deadline) {
            if (cond.test(screen())) return;
            Thread.sleep(20);
        }
        assertTrue(cond.test(screen()), "timed out waiting for " + what + ". Screen:\n" + screen());
    }

    private static void deleteTree(Path p) throws Exception {
        if (!Files.exists(p)) return;
        try (var walk = Files.walk(p)) {
            walk.sorted(java.util.Comparator.reverseOrder()).forEach(f -> f.toFile().delete());
        }
    }

    /** A Terminal-like host: its own display takes pointer input. */
    private record Host(UUID id, TerminalDisplay display) implements IComputerHost {
        @Override public UUID getComputerId() { return id; }
        @Override public MinecraftServer getServer() { return null; }
        @Override public void markDirty() {}
        @Override public void syncToClients() {}
        @Override public IFramebufferDisplay getFramebufferDisplay() { return display; }
        @Override public IRedstoneProvider getRedstoneProvider() { return null; }
        @Override public IWorldAccess getWorldAccess() { return null; }
        @Override public boolean acceptsPointerInput() { return true; }
    }
}
