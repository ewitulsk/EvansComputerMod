package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.api.IFramebufferDisplay;
import com.example.evanscomputermod.api.IRedstoneProvider;
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

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
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
        for (String optional : new String[]{"ssh.wasm", "sshd.wasm", "controllertest.wasm", "beep.wasm", "python.wasm", "lua.wasm", "gba.wasm", "rng_test.wasm", "vim.wasm"}) {
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

    /**
     * A display panel swap: the host asks the real kernel for another text
     * grid (on_display_resize) and the framebuffer the host reads changes size,
     * with the computer still running (P1 console resize).
     */
    @Test
    void consoleResizesWithoutReboot() throws Exception {
        assertEquals(160, display.getWidth());
        computer.requestConsoleResize(80, 25);
        waitFor(() -> display.getWidth() == 80 && display.getHeight() == 25, 5_000, "80x25 console");
        assertArrayEquals(new int[]{80, 25}, computer.grantedConsoleSize());
        computer.sendInput("echo resized-shell-ok\n");
        waitForScreen(s -> s.lines().anyMatch(l -> l.trim().equals("resized-shell-ok")), 10_000, "output at 80x25");
        computer.requestConsoleResize(160, 50);
        waitFor(() -> display.getWidth() == 160 && display.getHeight() == 50, 5_000, "160x50 console again");
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
     * Child programs get real entropy: ecm_host_abi::random (Wi-Fi SNonces,
     * traceroute ids) and the env.__getrandom_v03_custom import (getrandom
     * 0.3's custom backend) both return nonzero bytes that differ per call.
     * The import used to be auto-stubbed for children: "ok", buffer untouched.
     */
    @Test
    void childEntropyIsNonzeroAndFresh() throws Exception {
        assumeTrue(Files.exists(binDir.resolve("rng_test.wasm")),
                "rng_test not built (cargo build --release --target wasm32-wasip1 -p rng_test)");
        computer.sendInput("rng_test\n");
        java.util.regex.Pattern line = java.util.regex.Pattern.compile("rng (\\w+) rc=(-?\\d+) ([0-9a-f]{64})");
        waitForScreen(s -> line.matcher(s).results().count() >= 4, 10_000, "four rng lines");
        java.util.Map<String, String> draws = new java.util.HashMap<>();
        line.matcher(screen()).results().forEach(m -> {
            System.out.println("rng_test: " + m.group());
            assertEquals("0", m.group(2), m.group(1) + " reported an error");
            assertTrue(!m.group(3).matches("0+"), m.group(1) + " returned all zeros: " + m.group());
            draws.put(m.group(1), m.group(3));
        });
        assertEquals(4, draws.size(), "draws: " + draws.keySet());
        assertTrue(!draws.get("wrapper1").equals(draws.get("wrapper2")), "wrapper draws repeat");
        assertTrue(!draws.get("raw1").equals(draws.get("raw2")), "raw draws repeat");
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
        assertEquals(48000, audio.outputRate());
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
        int prompts = count(screen(), "/ > ");
        computer.sendInput("python t.py\n");
        java.util.List<Short> got = new java.util.ArrayList<>();
        long deadline = System.currentTimeMillis() + 240_000;
        // Wait for the prompt: print() may write a line in pieces.
        while (System.currentTimeMillis() < deadline && count(screen(), "/ > ") <= prompts) {
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

    /**
     * Vim on the real kernel and WASI host (Chicory): it opens a file full screen,
     * edits it with keys as the Terminal sends them (Enter as \n, Esc, Ctrl+C and
     * Ctrl+Z as its own keys on the alternate screen), follows a console resize
     * through the ?2048 report, writes the file and gives the shell's screen back.
     */
    @Test
    void vimEditsAFileOnTheRealHost() throws Exception {
        assumeTrue(Files.exists(binDir.resolve("vim.wasm")),
                "vim not built (WASI_SDK_PATH=... cargo build --release --target wasm32-wasip1 -p vim)");
        // ttimeout as defaults.vim sets it: without it Vim waits 'timeoutlen' (1 s)
        // after Esc, and a Ctrl+C in that second only leaves Insert mode.
        computer.sendInput("vim -u NONE -i NONE --cmd \"set ttimeout ttimeoutlen=100\" notes.txt\n");
        waitForScreen(s -> s.contains("\"notes.txt\" [New"), 60_000, "vim open");
        computer.sendInput("ihello world\nsecond line\u001b");
        waitForScreen(s -> s.lines().anyMatch(l -> l.trim().equals("second line")), 30_000, "typed lines");
        Thread.sleep(500); // the Esc timeout passes, as before a player's next key
        computer.sendInput("\u0003");
        waitForScreen(s -> s.contains("Type  :qa!  and press <Enter>"), 30_000, "Vim's Ctrl+C message");
        computer.requestConsoleResize(80, 25);
        waitFor(() -> display.getWidth() == 80, 5_000, "80x25 console");
        computer.sendInput(":echo &columns .. 'x' .. &lines\n");
        waitForScreen(s -> s.lines().anyMatch(l -> l.trim().startsWith("80x25")), 30_000, "Vim at 80x25");
        computer.sendInput("\u001a");
        waitForScreen(s -> s.contains("E9998"), 30_000, "Ctrl+Z is Vim's key");
        computer.sendInput(":wq\n");
        waitForScreen(s -> s.contains("/ > "), 30_000, "back at the shell");
        assertEquals("hello world\nsecond line\n", Files.readString(dataDir.resolve("notes.txt")));
    }

    /**
     * The Lua 5.2 interpreter runs a script: the C library's errors (setjmp/longjmp
     * over WASM exception handling) are caught by pcall, coroutines resume, `arg`
     * holds the arguments, files written with Lua's io are read back through the
     * shell module, and peripherals are listed and called.
     */
    @Test
    void luaRunsAScriptWithErrorsCoroutinesFilesAndPeripherals() throws Exception {
        assumeTrue(Files.exists(binDir.resolve("lua.wasm")),
                "lua not built (WASI_SDK_PATH=... cargo build --release --target wasm32-wasip1 -p lua)");
        var audio = new com.example.evanscomputermod.speaker.SpeakerAudio();
        hub.setWireless("left", new com.example.evanscomputermod.speaker.SpeakerPeripheral(
                new com.example.evanscomputermod.speaker.SpeakerPeripheral.Owner() {
                    @Override public com.example.evanscomputermod.speaker.SpeakerAudio audio() { return audio; }
                    @Override public void markVolumeChanged() {}
                    @Override public boolean playSound(String id, float v, float p) { return false; }
                }));
        Files.writeString(dataDir.resolve("t.lua"), String.join("\n",
                "print('VER', _VERSION, 7 / 2, bit32.band(12, 10), unpack({4, 5}))",
                "local ok, err = pcall(function() error('boom', 0) end)",
                "print('PCALL', ok, err)",
                "local co = coroutine.wrap(function(a) local b = coroutine.yield(a + 1) return b * 2 end)",
                "print('CO', co(1), co(21))",
                "print('ARG', arg[0], arg[1], select('#', ...))",
                "local f = assert(io.open('out.txt', 'w')) f:write('written by lua') f:close()",
                "print('FILE', shell.read_file('/out.txt'))",
                "for _, p in ipairs(peripheral.list()) do print('PERIPH', p.name, p.type) end",
                "local ok2, perr = pcall(peripheral.call, 'nowhere', 'status')",
                "print('NOPERIPH', ok2)",
                "error('uncaught at the end')",
                ""));
        int prompts = count(screen(), "/ > ");
        computer.sendInput("lua t.lua hello\n");
        waitForScreen(s -> count(s, "/ > ") > prompts, 60_000, "prompt after lua exits");
        String screen = screen();
        // print() separates values with tabs, which the terminal renders as columns.
        assertTrue(hasLine(screen, "VER", "Lua", "5.2", "3.5", "8", "4", "5"), screen);
        assertTrue(hasLine(screen, "PCALL", "false", "boom"), screen);
        assertTrue(hasLine(screen, "CO", "2", "42"), screen);
        assertTrue(hasLine(screen, "ARG", "t.lua", "hello", "1"), screen);
        assertTrue(hasLine(screen, "FILE", "written", "by", "lua"), screen);
        assertTrue(hasLine(screen, "PERIPH", "left", "speaker"), screen);
        assertTrue(hasLine(screen, "NOPERIPH", "false"), screen);
        assertTrue(screen.contains("uncaught at the end"), screen);
    }

    /**
     * The GBA emulator runs a test ROM on the terminal display with a
     * controller, plays its sound on a speaker, and quits on Guide.
     */
    @Test
    void gbaRunsATestRomWithControllerAndSpeaker() throws Exception {
        assumeTrue(Files.exists(binDir.resolve("gba.wasm")),
                "gba not built (cargo build --release --target wasm32-wasip1 -p gba)");
        Path rom = repoRoot().resolve("rust/wasm-programs/gba/tests/roms/arm.gba");
        Files.copy(rom, dataDir.resolve("arm.gba"));

        var audio = new com.example.evanscomputermod.speaker.SpeakerAudio();
        hub.setWireless("left", new com.example.evanscomputermod.speaker.SpeakerPeripheral(
                new com.example.evanscomputermod.speaker.SpeakerPeripheral.Owner() {
                    @Override public com.example.evanscomputermod.speaker.SpeakerAudio audio() { return audio; }
                    @Override public void markVolumeChanged() {}
                    @Override public boolean playSound(String id, float v, float p) { return false; }
                }));
        var pad = new com.example.evanscomputermod.controller.ControllerPeripheral(1);
        hub.setWireless("controller_1", pad);

        computer.sendInput("gba arm.gba\n");
        waitFor(() -> display.getDisplayMode() == 1 && display.getGfxWidth() == 240
                && display.getGfxHeight() == 160 && display.getPixelFormat() == TerminalDisplay.PIXEL_FORMAT_RGB565,
                30_000, "the GBA screen");
        // The test ROM draws "All tests passed": the frame stops being blank.
        waitFor(() -> {
            byte[] px = display.getPixelData();
            if (px == null) return false;
            for (int i = 0; i < px.length; i += 2) if (px[i] != px[0] || px[i + 1] != px[1]) return true;
            return false;
        }, 30_000, "the test ROM's text on screen");

        long samples = 0;
        long t0 = System.currentTimeMillis();
        while (System.currentTimeMillis() - t0 < 3000) {
            samples += audio.drain().length;
            Thread.sleep(20);
        }
        assertEquals(32768, audio.outputRate());
        assertTrue(samples > 32768 * 3 / 2, "audio keeps flowing: " + samples + " samples in 3 s");

        pad.update(new com.example.evanscomputermod.controller.ControllerState(
                com.example.evanscomputermod.controller.ControllerInput.Button.GUIDE.bit(), 0, 0, 0, 0, 0, 0));
        waitForScreen(s -> s.contains(" fps)."), 15_000, "gba exit");
        String line = screen().lines().filter(l -> l.contains(" fps).")).findFirst().orElse("");
        System.out.println("GBA on Chicory: " + line.trim());
        waitFor(() -> display.getDisplayMode() == 0, 5_000, "display handed back");
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

    /** A screen line whose whitespace-separated words include all of {@code words}. */
    private static boolean hasLine(String screen, String... words) {
        return screen.lines().anyMatch(l -> java.util.Arrays.asList(l.trim().split("\\s+"))
                .containsAll(java.util.Arrays.asList(words)));
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
    }
}
