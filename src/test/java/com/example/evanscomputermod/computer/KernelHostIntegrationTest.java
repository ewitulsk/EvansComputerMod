package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.api.IFramebufferDisplay;
import com.example.evanscomputermod.api.IRedstoneProvider;
import com.example.evanscomputermod.api.IVisualProgramming;
import com.example.evanscomputermod.api.IWorldAccess;
import com.example.evanscomputermod.wasm.WasmManager;
import com.example.evanscomputermod.wasm.chicory.ChicoryRuntimeProvider;
import net.minecraft.server.MinecraftServer;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.UUID;
import java.util.function.Predicate;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assumptions.assumeTrue;

/**
 * Layer-3 contract test: the real kernel (terminal_os.wasm) running inside
 * the real Java host ({@link ComputerInstance}) on the pure-Java Chicory
 * runtime. Catches host/kernel ABI drift (strict import linking runs in
 * loadModule), the event-driven worker loop, non-blocking job control and
 * Ctrl+T handling -- without starting Minecraft.
 *
 * <p>Needs the kernel and a few WASI programs built:
 * {@code cargo build --release --target wasm32-unknown-unknown -p terminal-os} and
 * {@code cargo build --release --target wasm32-wasip1 -p echo -p sleep -p ssh-client -p sshd}
 * (run in {@code rust/}).
 */
public class KernelHostIntegrationTest {

    private ComputerInstance computer;
    private TerminalDisplay display;
    private com.example.evanscomputermod.computer.peripheral.PeripheralHub hub;
    private Path binDir;
    private Path dataDir;

    private static Path repoRoot() {
        Path p = Path.of("").toAbsolutePath();
        while (p != null && !Files.exists(p.resolve("rust/Cargo.toml"))) {
            p = p.getParent();
        }
        return p;
    }

    @BeforeEach
    void boot() throws Exception {
        Path root = repoRoot();
        assumeTrue(root != null, "repository root not found");
        Path kernel = root.resolve("rust/target/wasm32-unknown-unknown/release/terminal_os.wasm");
        Path progs = root.resolve("rust/target/wasm32-wasip1/release");
        assumeTrue(Files.exists(kernel), "kernel not built: " + kernel);
        assumeTrue(Files.exists(progs.resolve("echo.wasm")) && Files.exists(progs.resolve("sleep.wasm")),
                "echo/sleep programs not built in " + progs);

        binDir = Files.createTempDirectory("ecm-wasm-bin");
        Files.copy(kernel, binDir.resolve("terminal_os.wasm"));
        Files.copy(progs.resolve("echo.wasm"), binDir.resolve("echo.wasm"));
        Files.copy(progs.resolve("sleep.wasm"), binDir.resolve("sleep.wasm"));
        for (String optional : new String[]{"ssh.wasm", "sshd.wasm", "controllertest.wasm", "beep.wasm", "python.wasm"}) {
            if (Files.exists(progs.resolve(optional))) {
                Files.copy(progs.resolve(optional), binDir.resolve(optional));
            }
        }
        WasmManager.setWasmBinPathForTesting(binDir);
        WasmManager.bind(new ChicoryRuntimeProvider().create());

        display = new TerminalDisplay();
        UUID id = UUID.randomUUID();
        dataDir = Path.of("computer-data", id.toString());
        FakeHost host = new FakeHost(id, display, new java.util.concurrent.atomic.AtomicReference<>());
        hub = new com.example.evanscomputermod.computer.peripheral.PeripheralHub(new com.example.evanscomputermod.computer.peripheral.PeripheralHub.Owner() {
            @Override public net.minecraft.world.level.Level level() { return null; }
            @Override public net.minecraft.core.BlockPos pos() { return net.minecraft.core.BlockPos.ZERO; }
            @Override public net.minecraft.core.Direction facing() { return net.minecraft.core.Direction.NORTH; }
            @Override public UUID computerId() { return id; }
            @Override public com.example.evanscomputermod.computer.peripheral.PeripheralEventBus events() {
                return computer == null ? null : computer.getPeripheralEvents();
            }
        });
        host.hub().set(hub);
        computer = new ComputerInstance(host, new byte[0][]);
        computer.loadModule("terminal_os"); // strict import check runs here
        computer.executeMain();
        computer.startWorkerThread();
        waitForScreen(s -> s.contains("Welcome to Terminal OS"), 10_000, "boot banner");
    }

    @AfterEach
    void shutdown() throws Exception {
        if (computer != null) computer.close();
        if (binDir != null) deleteTree(binDir);
        if (dataDir != null) deleteTree(dataDir);
    }

    @Test
    void childProgramOutputReachesScreenAndPromptReturns() throws Exception {
        int prompts = count(screen(), "/ > ");
        computer.sendInput("echo hello-from-child\n");
        // A line consisting only of the program's output (not the echoed command).
        waitForScreen(s -> s.lines().anyMatch(l -> l.trim().equals("hello-from-child")), 10_000, "child output");
        waitForScreen(s -> count(s, "/ > ") > prompts, 10_000, "prompt after child exit");
    }

    @Test
    void ctrlTStopsForegroundProgram() throws Exception {
        computer.sendInput("sleep 30\n");
        Thread.sleep(500);
        // Same sequence TerminalBlockEntity uses for Ctrl+T.
        computer.interrupt();
        computer.queueInterrupt(15, "{}");
        waitForScreen(s -> s.contains("^T - Program terminated"), 3_000, "Ctrl+T acknowledgement");
    }

    @Test
    void switchWithoutInterfacesExplainsWhy() throws Exception {
        computer.sendInput("switch on\n");
        waitForScreen(s -> s.contains("no network interfaces"), 5_000, "switch message");
    }

    /**
     * SSH end to end inside one computer, over loopback: sshd hosts a remote
     * shell session in the kernel, the ssh client relays it to this screen.
     */
    @Test
    void sshToOwnSshdRunsCommandsInARemoteSession() throws Exception {
        assumeTrue(Files.exists(binDir.resolve("ssh.wasm")) && Files.exists(binDir.resolve("sshd.wasm")),
                "ssh/sshd not built (cargo build --release --target wasm32-wasip1 -p ssh-client -p sshd)");
        computer.sendInput("sshd 2222 &\n");
        Thread.sleep(1500);
        computer.sendInput("ssh root@127.0.0.1:2222\n");
        waitForScreen(s -> s.contains("logged in as root"), 30_000, "remote login banner");
        computer.sendInput("echo over-ssh-ok\n");
        waitForScreen(s -> s.lines().anyMatch(l -> l.trim().equals("over-ssh-ok")), 15_000, "remote command output");
        computer.sendInput("exit\n");
        waitForScreen(s -> s.contains("Connection closed."), 15_000, "remote logout");
    }

    /**
     * A wireless controller drives a program end to end: the gamepad driver
     * reads it through the peripheral API, and the program draws it with the
     * double-buffered RGB565 display (init2 / blit / present at vblank); when
     * the program quits the display goes back to the kernel.
     */
    @Test
    void controllerDrivesAProgramOnADoubleBufferedDisplay() throws Exception {
        assumeTrue(Files.exists(binDir.resolve("controllertest.wasm")),
                "controllertest not built (cargo build --release --target wasm32-wasip1 -p controllertest)");
        var pad = new com.example.evanscomputermod.controller.ControllerPeripheral(1);
        hub.setWireless("controller_1", pad);

        computer.sendInput("controllertest\n");
        waitFor(() -> display.getDisplayMode() == 1 && display.getGfxWidth() == 320
                && display.getPixelFormat() == TerminalDisplay.PIXEL_FORMAT_RGB565, 15_000, "RGB565 display");

        // A is drawn at (225, 102): dark when released, green when pressed.
        int green = ((63 >> 3) << 11) | ((185 >> 2) << 5) | (80 >> 3);
        waitFor(() -> pixel565(225, 102) != green, 5_000, "A drawn released");
        pad.update(new com.example.evanscomputermod.controller.ControllerState(
                com.example.evanscomputermod.controller.ControllerInput.Button.A.bit(), 0, 0, 0, 0, 0, 0));
        waitFor(() -> pixel565(225, 102) == green, 5_000, "A lit after pressing it");

        int quit = com.example.evanscomputermod.controller.ControllerInput.Button.BACK.bit()
                | com.example.evanscomputermod.controller.ControllerInput.Button.START.bit();
        pad.update(new com.example.evanscomputermod.controller.ControllerState(quit, 0, 0, 0, 0, 0, 0));
        waitForScreen(s -> s.contains("controllertest: bye."), 10_000, "program exit");
        waitFor(() -> display.getDisplayMode() == 0, 5_000, "display handed back to the kernel");
    }

    /** A program plays a tone through /dev/audio; the speaker receives exactly its samples. */
    @Test
    void beepWritesPcmToTheSpeakerDevice() throws Exception {
        assumeTrue(Files.exists(binDir.resolve("beep.wasm")),
                "beep not built (cargo build --release --target wasm32-wasip1 -p beep)");
        computer.sendInput("beep\n");
        waitForScreen(s -> s.contains("no speaker"), 10_000, "no-speaker message");

        var audio = new com.example.evanscomputermod.speaker.SpeakerAudio();
        var speaker = new com.example.evanscomputermod.speaker.SpeakerPeripheral(
                new com.example.evanscomputermod.speaker.SpeakerPeripheral.Owner() {
                    @Override public com.example.evanscomputermod.speaker.SpeakerAudio audio() { return audio; }
                    @Override public void markVolumeChanged() {}
                    @Override public boolean playSound(String id, float v, float p) { return false; }
                });
        hub.setWireless("left", speaker);

        int prompts = count(screen(), "/ > ");
        java.util.List<Short> got = new java.util.ArrayList<>();
        computer.sendInput("beep 1000 100 --speaker left\n");
        long deadline = System.currentTimeMillis() + 15_000;
        while (System.currentTimeMillis() < deadline && count(screen(), "/ > ") <= prompts) {
            for (short v : audio.drain()) got.add(v);
            Thread.sleep(10);
        }
        Thread.sleep(50);
        for (short v : audio.drain()) got.add(v);
        assertTrue(count(screen(), "/ > ") > prompts, "beep didn't finish. Screen:\n" + screen());
        assertEquals(48000, audio.rate());
        assertEquals(4800, got.size(), "100 ms at 48 kHz");
        assertTrue(got.stream().anyMatch(v -> Math.abs(v) > 5000), "a loud tone");
    }

    /** The Python `audio` and `controller` modules, over the same devices. */
    @Test
    void pythonAudioAndControllerModules() throws Exception {
        assumeTrue(Files.exists(binDir.resolve("python.wasm")),
                "python not built (cargo build --release --target wasm32-wasip1 -p python)");
        var audio = new com.example.evanscomputermod.speaker.SpeakerAudio();
        hub.setWireless("left", new com.example.evanscomputermod.speaker.SpeakerPeripheral(
                new com.example.evanscomputermod.speaker.SpeakerPeripheral.Owner() {
                    @Override public com.example.evanscomputermod.speaker.SpeakerAudio audio() { return audio; }
                    @Override public void markVolumeChanged() {}
                    @Override public boolean playSound(String id, float v, float p) { return false; }
                }));
        var pad = new com.example.evanscomputermod.controller.ControllerPeripheral(1);
        hub.setWireless("controller_1", pad);
        pad.update(new com.example.evanscomputermod.controller.ControllerState(
                com.example.evanscomputermod.controller.ControllerInput.Button.B.bit(), 0, 127, 0, 0, 0, 0));
        Files.writeString(dataDir.resolve("t.py"), String.join("\n",
                "import audio, controller",
                "s = audio.open('left')",
                "s.tone(440, 0.05)",
                "st = s.status()",
                "print('RATE', st['rate'], 'BITS', st['bits'])",
                "p = controller.find()",
                "print('PAD', p.player, p.is_down('b'), p.axis('ly'), p.buttons())",
                ""));
        computer.sendInput("python t.py\n");
        java.util.List<Short> got = new java.util.ArrayList<>();
        long deadline = System.currentTimeMillis() + 240_000;
        while (System.currentTimeMillis() < deadline && !screen().contains("PAD ") && !screen().contains("Error")) {
            for (short v : audio.drain()) got.add(v);
            Thread.sleep(50);
        }
        String screen = screen();
        assertTrue(screen.contains("RATE 8000 BITS 8"), screen);
        assertTrue(screen.contains("PAD 1 True 1.0 ['b']"), screen);
        Thread.sleep(50);
        for (short v : audio.drain()) got.add(v);
        assertEquals(400, got.size(), "0.05 s at 8 kHz");
    }

    private int pixel565(int x, int y) {
        byte[] px = display.getPixelData();
        int i = (y * display.getGfxWidth() + x) * 2;
        if (px == null || display.getPixelFormat() != TerminalDisplay.PIXEL_FORMAT_RGB565 || i + 1 >= px.length) return -1;
        return (px[i] & 0xFF) | ((px[i + 1] & 0xFF) << 8);
    }

    private static void waitFor(java.util.function.BooleanSupplier cond, long timeoutMs, String what) throws InterruptedException {
        long deadline = System.currentTimeMillis() + timeoutMs;
        while (System.currentTimeMillis() < deadline) {
            if (cond.getAsBoolean()) return;
            Thread.sleep(20);
        }
        assertTrue(cond.getAsBoolean(), "timed out waiting for " + what);
    }

    // ------------------------------------------------------------ helpers

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

    private static int count(String s, String needle) {
        int n = 0;
        for (int i = s.indexOf(needle); i >= 0; i = s.indexOf(needle, i + 1)) n++;
        return n;
    }

    private static void deleteTree(Path p) throws Exception {
        if (!Files.exists(p)) return;
        try (var walk = Files.walk(p)) {
            walk.sorted(java.util.Comparator.reverseOrder()).forEach(f -> f.toFile().delete());
        }
    }

    /** Minimal host: no world, no redstone, a plain TerminalDisplay. */
    private record FakeHost(UUID id, TerminalDisplay display,
                            java.util.concurrent.atomic.AtomicReference<com.example.evanscomputermod.computer.peripheral.PeripheralHub> hub)
            implements IComputerHost {
        @Override public com.example.evanscomputermod.computer.peripheral.PeripheralHub getPeripheralHub() { return hub.get(); }
        @Override public UUID getComputerId() { return id; }
        @Override public MinecraftServer getServer() { return null; }
        @Override public void markDirty() {}
        @Override public void syncToClients() {}
        @Override public IFramebufferDisplay getFramebufferDisplay() { return display; }
        @Override public IRedstoneProvider getRedstoneProvider() { return null; }
        @Override public IWorldAccess getWorldAccess() { return null; }
        @Override public IVisualProgramming getVisualProgramming() { return null; }
    }
}
