package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.EcmConfig;
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.ui.Compositor;
import com.example.evanscomputermod.computer.ui.UiConnection;
import com.example.evanscomputermod.computer.ui.UiManager;
import com.example.evanscomputermod.computer.ui.proto.UiProto;
import com.example.evanscomputermod.radio.sdr.RadioSdrContent;
import com.example.evanscomputermod.radio.sdr.SdrBlock;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.Blocks;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.WeakHashMap;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * GUI scenarios (1.21.1), spawnable with {@code /ecm scenario spawn <name>}
 * and run as the {@code ecm_gui} GameTests. Built and operated by a
 * {@link ScenarioPlayer}: the Terminal and the SDR are placed by right-clicks,
 * files are typed in from Python, and every key, click and line of text goes
 * through the client's input packets ({@code TerminalInputPacket},
 * {@code KeyInputPacket}, {@code MouseInputPacket}) and their server handlers.
 * The steps that are not player-equivalent are named in each scenario's notes
 * and in docs/SCENARIO_EXCEPTIONS.md.
 *
 * <p>{@code gui_basics}: a Terminal with a Standard SDR on its east side. Its
 * {@code /etc/ui.toml} tells the test shell to open Notes, {@code gba} (a test
 * ROM) and {@code waterfall} (on the SDR). Typing {@code gui} starts the
 * compositor; the test shell tiles the three windows under its bar. Then:
 * Ctrl+Alt+Backspace to the console (the GUI keeps running), the shell killed
 * from the console with {@code ps} and {@code kill} (safe mode, the shell back
 * within a second), back to the GUI, and a control: a client sending garbage
 * loses only its own connection.
 * <pre>
 *   y=0 floor x -3..3, z -2..2; Terminal (0,1,0) facing north; SDR (1,1,0) facing south.
 * </pre>
 */
public final class GuiScenarios {
    private static final BlockPos PC = new BlockPos(0, 1, 0);
    private static final BlockPos SDR = PC.east();
    /** /etc/ui.toml of gui_basics: the test shell opens Notes, gba and waterfall. */
    static final String[] UI_TOML = {
            "# The test shell opens these at start (scenario gui_basics).",
            "autostart = false",
            "shell = \"test-shell\"",
            "shell_args = [\"--launch\", \"notes\", \"--launch\", \"gba:arm.gba\", \"--launch\", \"waterfall:100M:--size:200x120:--rate:48000\"]",
            "theme = \"ecm-default\"",
            "[screen]",
            "mode = \"mirror\"",
    };
    /** gui_client_render: the test shell opens Notes only (gba is launched later by the scenario). */
    static final String[] UI_TOML_NOTES = {
            "# The test shell opens Notes (scenario gui_client_render).",
            "autostart = false",
            "shell = \"test-shell\"",
            "shell_args = [\"--launch\", \"notes\"]",
            "theme = \"ecm-default\"",
    };
    /** The test ROM, a binary file: copied into the computer's folder (docs/SCENARIO_EXCEPTIONS.md). */
    static final String ROM_DIR = "/evanscomputermod/scenarios/gui/";

    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();
    /** When each run killed its shell (for the "back within a second" check). */
    private static final Map<ScenarioRun, Long> KILLED_AT = new WeakHashMap<>();
    private static final Map<ScenarioRun, UiConnection> BROKEN = new WeakHashMap<>();
    /** gui_client_render: the player whose client draws the GUI, and the typing run. */
    private static final Map<ScenarioRun, com.example.evanscomputermod.testing.SceneViewerMirror> VIEWER = new WeakHashMap<>();
    private static final Map<ScenarioRun, long[]> TYPING = new WeakHashMap<>();
    /** Keystrokes and their rate in the typing check (about 100 words a minute). */
    static final int KEYS = 24, KEYS_PER_SECOND = 8;
    /** A {@code ps} row of the test shell: pid, state, name. */
    static final Pattern PS_SHELL = Pattern.compile("(?m)^\\s*(\\d+)\\s+\\S+\\s+\\S*test-shell");


    private GuiScenarios() {}

    static ComputerInstance computer(ScenarioRun r) {
        TerminalBlockEntity be = r.terminal("pc");
        return be == null ? null : be.getComputer();
    }

    static UiManager ui(ScenarioRun r) {
        ComputerInstance c = computer(r);
        return c == null ? null : c.getUiManager();
    }

    static Compositor comp(ScenarioRun r) {
        UiManager u = ui(r);
        return u == null ? null : u.compositor();
    }

    static List<Compositor.WindowInfo> mapped(ScenarioRun r) {
        Compositor c = comp(r);
        if (c == null) return List.of();
        return c.windows().stream().filter(w -> w.mapped() && w.placed()).toList();
    }

    /** Null when every window is drawn, side by side; otherwise what's wrong. */
    static String tiled(ScenarioRun r, int n) {
        List<Compositor.WindowInfo> m = new ArrayList<>(mapped(r));
        if (m.size() != n) return m.size() + " of " + n + " windows drawn";
        m.sort(java.util.Comparator.comparingInt(Compositor.WindowInfo::x));
        for (int i = 0; i + 1 < m.size(); i++) {
            if (m.get(i).x() + m.get(i).w() > m.get(i + 1).x()) return "windows overlap: " + m;
        }
        return null;
    }

    /** Null if the Terminal display shows varied pixels inside window {@code app}'s rectangle. */
    static String drawing(ScenarioRun r, String app) {
        Compositor.WindowInfo w = mapped(r).stream().filter(x -> x.appId().equals(app)).findFirst().orElse(null);
        if (w == null) return app + " has no window";
        TerminalDisplay d = r.terminal("pc").getDisplay();
        byte[] px;
        synchronized (d) {
            if (d.getDisplayMode() != 1 || d.getGfxWidth() != 640 || d.getPixelFormat() != TerminalDisplay.PIXEL_FORMAT_RGBA8888) {
                return "the Terminal doesn't show the GUI";
            }
            px = d.getPixelData().clone();
        }
        int first = -1;
        for (int y = w.y() + 2; y < w.y() + w.h() - 2; y += 2) {
            for (int x = w.x() + 2; x < w.x() + w.w() - 2; x += 2) {
                int i = (y * 640 + x) * 4;
                int c = (px[i] & 0xFF) << 16 | (px[i + 1] & 0xFF) << 8 | (px[i + 2] & 0xFF);
                if (first < 0) first = c;
                else if (c != first) return null;
            }
        }
        return app + "'s window is one flat colour";
    }

    // ================================================================ the player's keyboard and mouse

    /** A left click at a device pixel of the pc's screen (MouseInputPacket, through its handler). */
    static void click(ScenarioRun r, int[] at) {
        r.click("pc", at[0], at[1]);
    }

    /** A key chord for the GUI (raw key events). */
    static void key(ScenarioRun r, int hid, int mods) {
        r.pressKey("pc", hid, mods, null);
    }

    /** Enter, as the client sends it (the raw key and the console's newline). */
    static void enter(ScenarioRun r) {
        r.pressKey("pc", ScenarioPlayer.HID_ENTER, 0, "\n");
    }

    /** Type text at the pc's keyboard (the focused window gets it). */
    static void type(ScenarioRun r, String text) {
        r.typeText("pc", text);
    }

    /** Ctrl+Alt+Backspace, as the player presses it: switch between the console and the GUI. */
    static void chord(ScenarioRun r) {
        r.pressKey("pc", ScenarioPlayer.HID_BACKSPACE, UiManager.SWITCH_MODS, null);
    }

    // ================================================================ gui_basics

    private static Scenario guiBasics() {
        var b = Scenario.builder("gui_basics",
                        "the GUI on a Terminal: the test shell tiles Notes, gba and waterfall; the console switch; the shell killed"
                                + " from the console (safe mode)")
                .asPlayer()
                .host("pc", PC, "-")
                .decor(new Desk())
                .realTime()
                .note("Setup (done for you): a Standard SDR placed east of the Terminal; the test ROM arm.gba copied into the"
                        + " computer's folder <world>/computer-data/<id>/ (a binary file: the one step that isn't typed, see"
                        + " docs/SCENARIO_EXCEPTIONS.md).")
                .note("Write /etc/ui.toml from Python: the test shell opens Notes, gba and waterfall");
        PlayerKit.typeFile(b, "pc", "/etc/ui.toml", UI_TOML);
        return b.note("Control: without the GUI there is no compositor, so a GUI app refuses to run")
                .send("pc", "notes")
                .expect("pc", "no GUI is running", "notes says there is no GUI")
                .note("Start the GUI: /etc/ui.toml makes the test shell open Notes, gba and waterfall")
                .send("pc", "gui")
                .expect("pc", "^gui: started", "the compositor and the test shell started")
                .await(r -> comp(r) != null && comp(r).shellConnected() ? null : "waiting for the shell", "the test shell connected", 15_000)
                .await(r -> tiled(r, 3), "three windows tiled side by side under the shell's bar", 30_000)
                .await(r -> drawing(r, "gba"), "gba draws the test ROM in its window (legacy gfx, unchanged)", 30_000)
                .await(r -> drawing(r, "waterfall"), "waterfall draws the SDR spectrum in its window", 30_000)
                .mutate(r -> EvansComputerMod.LOGGER.info("[gui_basics] {}", ui(r).status().strip()), "log the windows")
                .note("Ctrl+Alt+Backspace: the console comes back; the GUI keeps running")
                .mutate(GuiScenarios::chord, "press Ctrl+Alt+Backspace")
                .await(r -> !ui(r).guiVisible() && r.terminal("pc").getDisplay().getDisplayMode() == 0 ? null : "GUI still shown",
                        "the console is shown", 5_000)
                .send("pc", "echo console-ok")
                .expect("pc", "^console-ok", "the console takes keys again")
                .await(r -> ui(r).isRunning() && mapped(r).size() == 3 ? null : "GUI lost windows", "the GUI still runs its three apps", 1_000)
                .note("Kill the shell from the console: ps, then kill <its pid>. Safe mode keeps the windows, the shell restarts")
                .send("pc", "ps")
                .expect("pc", "test-shell", "ps lists the test shell")
                .sendFn("pc", r -> {
                    String out = r.latestOutput("pc");
                    Matcher m = PS_SHELL.matcher(out == null ? "" : out);
                    if (!m.find()) throw new IllegalStateException("ps doesn't list the test shell:\n" + out);
                    KILLED_AT.put(r, System.currentTimeMillis());
                    return "kill " + m.group(1);
                }, "kill <the test shell's pid, from ps>")
                .expect("pc", "^Process killed", "the shell's process is killed", "^Failed to kill")
                .await(r -> comp(r).safeMode() ? null : "not in safe mode", "safe mode (focused window maximized, plain bar)", 2_000)
                .await(r -> comp(r).shellConnected() && !comp(r).safeMode() ? null : "shell still away", "the shell back", 3_000)
                .await(r -> {
                    long ms = System.currentTimeMillis() - KILLED_AT.getOrDefault(r, 0L);
                    return ms <= 1_500 ? null : "the shell took " + ms + " ms to come back";
                }, "the shell came back within a second (plus a tick of slack)", 100)
                .await(r -> tiled(r, 3), "the new shell tiled the windows again", 10_000)
                .mutate(GuiScenarios::chord, "press Ctrl+Alt+Backspace again")
                .await(r -> ui(r).guiVisible() ? null : "console still shown", "the GUI is shown again", 5_000)
                .await(r -> drawing(r, "gba"), "gba still drawing", 10_000)
                .note("Control: a client that sends garbage loses only its own connection [exception: the broken client is opened"
                        + " by the scenario, since no program a player can run sends malformed protocol bytes]")
                .mutate(r -> {
                    int[] err = new int[1];
                    UiConnection c = comp(r).open(9999, UiProto.Role.APP, err);
                    BROKEN.put(r, c);
                    if (c != null) c.send(new byte[]{(byte) 0xEE, (byte) 0xEE, 0, 0, 0, 0, 0, 0});
                }, "open a UI connection and send an unknown opcode")
                .await(r -> {
                    UiConnection c = BROKEN.get(r);
                    if (c == null || !c.isClosed() || c.closeReason() == null) return "the broken connection is still open";
                    if (c.closeReason().code != UiProto.ErrorCode.UNKNOWN_OPCODE) return "closed with " + c.closeReason().code;
                    return comp(r).shellConnected() ? tiled(r, 3) : "the shell lost its connection too";
                }, "only that connection closed (Error unknown_opcode); the shell and all three apps are fine", 2_000)
                .timeLimit(60_000)
                .build();
    }

    // ================================================================ gui_desktop

    /** /etc/ui.toml of gui_desktop's computer: only "start the GUI at boot" (the desktop is the default shell). */
    static final String DESKTOP_TOML = "autostart = true";
    /** A program with no window that asks for the clipboard (the clipboard control). */
    static final String[] SNOOP_PY = {
            "import _ui",
            "h = _ui.connect(\"dev.pat.snoop\", 0)",
            "_ui.send(h, \"ClipboardRequest\", {\"mime\": \"text/plain\"})",
            "got = None",
            "for i in range(30):",
            "    for e in _ui.step(h, 100) or []:",
            "        if e[0] == \"message\" and e[1] == \"ClipboardData\":",
            "            got = _ui.json_loads(e[2])",
            "    if got is not None:",
            "        break",
            "if got is None:",
            "    print(\"snoop: no answer\")",
            "elif got.get(\"mime\", \"\") == \"\":",
            "    print(\"snoop: refused\")",
            "else:",
            "    print(\"snoop: GOT \" + str(got.get(\"data\")))",
    };
    static final String NETCHECK = "dev.ecm.netcheck";
    static final String NETCHECK_ASKS = "Allow Net Check to use network?";
    private static final Map<ScenarioRun, String> ADDRESS = new WeakHashMap<>();

    /** App windows (not the desktop's frames), drawn, placed and shown. */
    static List<Compositor.WindowInfo> appWindows(ScenarioRun r) {
        Compositor c = comp(r);
        if (c == null) return List.of();
        return c.windows().stream().filter(w -> !w.shell() && w.mapped() && w.placed() && w.visible()).toList();
    }

    static Compositor.WindowInfo appWindow(ScenarioRun r, String appId) {
        return appWindows(r).stream().filter(w -> w.appId().equals(appId)).findFirst().orElse(null);
    }

    /** Null if {@code appId}'s window shows a text containing {@code text}. */
    static String shows(ScenarioRun r, String appId, String text) {
        Compositor.WindowInfo w = appWindow(r, appId);
        if (w == null) return appId + " has no window";
        List<String> texts = comp(r).textsOf(w.id());
        if (texts.stream().anyMatch(t -> t.contains(text))) return null;
        return appId + " doesn't show '" + text + "' (it shows " + texts + ")";
    }

    static long count(ScenarioRun r, String appId, String text) {
        Compositor.WindowInfo w = appWindow(r, appId);
        return w == null ? 0 : comp(r).textsOf(w.id()).stream().filter(t -> t.contains(text)).count();
    }

    /** Where a drawn text is: in an app's window, the taskbar ("taskbar"), a layer of the desktop, or a popup ("popup"); null if not drawn. */
    static int[] locate(ScenarioRun r, String where, String text) {
        Compositor c = comp(r);
        if (c == null) return null;
        if (where.equals("popup")) return c.locatePopupText(text, false);
        if (where.equals("taskbar") || where.equals("permission")) return c.locateLayerText(where, text, false);
        Compositor.WindowInfo w = appWindow(r, where);
        return w == null ? null : c.locateText(w.id(), text, false);
    }

    /** Click a drawn text (found where the player sees it), through the mouse packet. */
    static void clickText(ScenarioRun r, String where, String text) {
        int[] at = locate(r, where, text);
        if (at == null) throw new IllegalStateException("can't see '" + text + "' in " + where);
        r.say("§7  click '" + text + "' in " + where + " at " + at[0] + "," + at[1]);
        click(r, at);
    }

    /** The composited GUI as {@code <screenshots>/<name>.png} (the runner keeps it with the receipt). */
    static void snapshot(ScenarioRun r, String name) {
        Compositor c = comp(r);
        if (c == null) return;
        try {
            int w = c.display().deviceW(), h = c.display().deviceH();
            byte[] f = c.frameRgba();
            java.awt.image.BufferedImage img = new java.awt.image.BufferedImage(w, h, java.awt.image.BufferedImage.TYPE_INT_RGB);
            for (int y = 0; y < h; y++)
                for (int x = 0; x < w; x++) {
                    int i = (y * w + x) * 4;
                    img.setRGB(x, y, (f[i] & 0xFF) << 16 | (f[i + 1] & 0xFF) << 8 | (f[i + 2] & 0xFF));
                }
            Path dir = com.example.evanscomputermod.testing.ScreenCapture.dir();
            Files.createDirectories(dir);
            javax.imageio.ImageIO.write(img, "png", dir.resolve(name + ".png").toFile());
            r.say("§7  wrote " + dir.resolve(name + ".png") + " (" + w + "x" + h + ")");
        } catch (IOException e) {
            r.say("§c  snapshot failed: " + e);
        }
    }

    /** Null once {@code text} is drawn where {@link #clickText} would look. */
    static String visible(ScenarioRun r, String where, String text) {
        if (comp(r) == null) return "no GUI";
        return locate(r, where, text) != null ? null : "'" + text + "' isn't drawn in " + where + " yet";
    }

    /** The composited colour (0xRRGGBB) at a device pixel. */
    static int pixel(ScenarioRun r, int x, int y) {
        Compositor c = comp(r);
        byte[] f = c.frameRgba();
        int i = (y * c.display().deviceW() + x) * 4;
        return (f[i] & 0xFF) << 16 | (f[i + 1] & 0xFF) << 8 | (f[i + 2] & 0xFF);
    }

    /** Launch an app from the taskbar's launcher: the Apps button, then its entry. */
    static Scenario.Builder launch(Scenario.Builder b, String label, String appId) {
        return b.await(r -> visible(r, "taskbar", "Apps"), "the taskbar's Apps button is drawn", 10_000)
                .mutate(r -> clickText(r, "taskbar", "Apps"), "click Apps on the taskbar")
                .await(r -> visible(r, "popup", label), "the launcher lists " + label, 5_000)
                .mutate(r -> clickText(r, "popup", label), "click " + label + " in the launcher")
                .await(r -> appWindow(r, appId) != null ? null : "no " + appId + " window yet", label + " opens in a frame", 20_000);
    }

    /** Type a command into the focused Terminal app and press Enter. */
    static Scenario.Builder terminalCommand(Scenario.Builder b, String line) {
        return b.mutate(r -> type(r, line), "type in the Terminal: " + line)
                .mutate(GuiScenarios::enter, "Enter");
    }

    /** Net Check's running instance on the pc, or null. */
    static com.example.evanscomputermod.computer.apps.AppInstance netCheck(ScenarioRun r) {
        com.example.evanscomputermod.computer.apps.AppInstance last = null;
        for (var i : computer(r).getAppManager().running()) if (i.appId().equals(NETCHECK)) last = i;
        return last;
    }

    private static Scenario guiDesktop() {
        List<BlockPos> floor = new ArrayList<>(PlayerKit.box(-2, 0, -1, 7, 0, 11));
        floor.removeAll(WifiScenarios.CAFE_CABLE);
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(WifiScenarios.CAFE_CABLE);
        foot.add(WifiScenarios.CAFE_AP);
        int lightBg = 0xf3f4f6;
        var b = Scenario.builder("gui_desktop",
                        "the desktop: autostart boots to it; Settings joins Wi-Fi without a command; Terminal, Files and the Editor"
                                + " open from the launcher; copy in the Editor, paste in the Terminal; the Light theme restyles every app;"
                                + " a real app's permission prompt; controls: a program without a window can't read the clipboard")
                .asPlayer()
                .realTime()
                .host("router", WifiScenarios.CAFE_ROUTER, "192.168.60.1/24")
                .host("pc", WifiScenarios.CAFE_LAPTOP, "-")
                .decor(PlayerKit.decor(foot, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()), r -> {
                    for (BlockPos p : WifiScenarios.CAFE_CABLE) r.player().place(com.example.evanscomputermod.block.ModBlocks.NETWORK_CABLE.get(), r.abs(p), Direction.UP);
                    PlayerKit.accessPoint(r, WifiScenarios.CAFE_AP, Direction.NORTH, WifiScenarios.CAFE_SSID, WifiScenarios.CAFE_PASS, 6, 20);
                    PlayerKit.wifiModules(r, "pc");
                }, r -> {
                    UiManager u = ui(r);
                    if (u != null) u.stop();
                }))
                .timeLimit(60_000)
                .note("Setup (done for you): a router computer cabled to an Access Point (SSID " + WifiScenarios.CAFE_SSID + ", WPA2 \""
                        + WifiScenarios.CAFE_PASS + "\", channel 6); a pc with a Wi-Fi Module.")
                .send("router", "ifconfig eth0 192.168.60.1/24")
                .expect("router", "eth0: inet 192\\.168\\.60\\.1/24", "router has 192.168.60.1");
        b.note("Files, typed from Python: the router's DHCP pool (/etc/dhcpd.conf); on the pc /etc/ui.toml (only autostart = true),"
                + " a note in /home and snoop.py for the clipboard control");
        Map<String, Map<String, String>> files = new LinkedHashMap<>();
        files.put("router", Map.of("/etc/dhcpd.conf", WifiScenarios.CAFE_POOL));
        Map<String, String> pc = new LinkedHashMap<>();
        pc.put("/etc/ui.toml", DESKTOP_TOML);
        pc.put("/home/notes.txt", "Buy redstone");
        pc.put("/home/snoop.py", String.join("\n", SNOOP_PY));
        files.put("pc", pc);
        PlayerKit.typeFiles(b, files);
        b.send("router", "dhcpd &")
                .expect("router", "serving eth0 192\\.168\\.60\\.10-192\\.168\\.60\\.100/24 as 192\\.168\\.60\\.1", "dhcpd serving the pool");
        b.note("Restart the pc (break it, pick it up, place it back, open it): it boots straight to the desktop");
        PlayerKit.powerCycle(b, "pc");
        b.note("The pc booted straight to the desktop (autostart): wallpaper, taskbar, no command typed")
                .await(r -> comp(r) != null && comp(r).shellConnected() ? null : "no desktop yet", "the desktop shell is connected", 20_000)
                .await(r -> comp(r).layerTexts("taskbar").contains("Apps") ? null : "no taskbar yet", "the taskbar shows Apps and the clock", 10_000)
                .note("Settings: scan, pick the network, type the password, Join");
        launch(b, "Settings", "settings")
                .await(r -> shows(r, "settings", "Not connected"), "Settings' Wi-Fi page says Not connected", 10_000)
                .mutate(r -> clickText(r, "settings", "Scan"), "click Scan")
                .await(r -> visible(r, "settings", WifiScenarios.CAFE_SSID), "the scan lists " + WifiScenarios.CAFE_SSID, 15_000)
                .mutate(r -> clickText(r, "settings", WifiScenarios.CAFE_SSID), "click " + WifiScenarios.CAFE_SSID)
                .await(r -> visible(r, "settings", "Join " + WifiScenarios.CAFE_SSID), "a password field and Join", 5_000)
                .mutate(r -> type(r, WifiScenarios.CAFE_PASS), "type the password")
                .mutate(r -> snapshot(r, "gui_desktop_wifi"), "snapshot: the Wi-Fi page before Join")
                .mutate(r -> clickText(r, "settings", "Join " + WifiScenarios.CAFE_SSID), "click Join")
                .await(r -> shows(r, "settings", "Connected. Address 192.168.60."), "Settings: Connected, with a DHCP address", 35_000)
                .await(r -> {
                    Compositor.WindowInfo w = appWindow(r, "settings");
                    for (String t : comp(r).textsOf(w.id())) {
                        var m = Pattern.compile("Address (192\\.168\\.60\\.\\d+)/24").matcher(t);
                        if (m.find()) {
                            ADDRESS.put(r, m.group(1));
                            return null;
                        }
                    }
                    return "no address shown";
                }, "read the pc's address from Settings", 2_000)
                .sendFn("router", r -> "ping " + ADDRESS.get(r) + " -n 1", "ping <the pc's address> -n 1")
                .expect("router", "^1 packets sent, 1 received", "the router reaches the pc over Wi-Fi")
                .note("Terminal, Files and the Text Editor from the launcher");
        launch(b, "Terminal", "terminal")
                .await(r -> shows(r, "terminal", "logged in as"), "the Terminal runs a shell session", 15_000);
        launch(b, "Files", "files")
                .await(r -> shows(r, "files", "home"), "Files lists the disk", 10_000);
        launch(b, "Text Editor", "editor")
                .await(r -> shows(r, "editor", "New file"), "the Text Editor is ready", 10_000)
                .note("Copy in the Editor, paste in the Terminal")
                .mutate(r -> type(r, "echo pasted-into-terminal"), "type in the Editor")
                .await(r -> shows(r, "editor", "echo pasted-into-terminal"), "the Editor shows the text", 5_000)
                .mutate(r -> {
                    key(r, ScenarioPlayer.hidOf('a'), UiProto.Mods.CTRL);
                    key(r, ScenarioPlayer.hidOf('c'), UiProto.Mods.CTRL);
                }, "Ctrl+A, Ctrl+C")
                .await(r -> comp(r).clipboardText().equals("echo pasted-into-terminal") ? null : "clipboard: '" + comp(r).clipboardText() + "'",
                        "the clipboard holds the Editor's text", 5_000)
                .mutate(r -> clickText(r, "taskbar", "Terminal"), "click the Terminal's taskbar button")
                .await(r -> appWindow(r, "terminal") != null && appWindow(r, "terminal").focused() ? null : "Terminal not focused", "the Terminal has focus", 5_000)
                .note("Control: a program without a window (python /home/snoop.py) asks for the clipboard and gets nothing");
        terminalCommand(b, "python /home/snoop.py")
                .await(r -> {
                    if (count(r, "terminal", "snoop: GOT") > 0) {
                        r.fail("control: a program without a focused window read the clipboard");
                        return null;
                    }
                    return shows(r, "terminal", "snoop: refused");
                }, "snoop.py is refused (ClipboardData with no type)", 15_000)
                .mutate(r -> key(r, ScenarioPlayer.hidOf('v'), UiProto.Mods.CTRL | UiProto.Mods.SHIFT), "Ctrl+Shift+V")
                .await(r -> shows(r, "terminal", "echo pasted-into-terminal"), "the command is pasted into the Terminal", 5_000)
                .mutate(GuiScenarios::enter, "Enter")
                .await(r -> count(r, "terminal", "pasted-into-terminal") >= 2 ? null : "no output yet", "it ran: the output is in the Terminal", 10_000)
                .note("Theme: Settings > Appearance > Light restyles every open app")
                .mutate(r -> clickText(r, "taskbar", "Settings"), "click Settings on the taskbar")
                .await(r -> visible(r, "settings", "Appearance"), "Settings is in front", 5_000)
                .mutate(r -> clickText(r, "settings", "Appearance"), "click Appearance")
                .await(r -> visible(r, "settings", "Light"), "the themes are listed", 5_000)
                .mutate(r -> clickText(r, "settings", "Light"), "click Light")
                .await(r -> comp(r).themeName().equals("ecm-light") ? null : "theme " + comp(r).themeName(), "the desktop applied ecm-light", 10_000)
                .await(r -> {
                    for (String a : List.of("terminal", "files", "editor", "settings")) {
                        Compositor.WindowInfo w = appWindow(r, a);
                        if (w == null) return a + " closed";
                        int px = pixel(r, w.x() + w.w() - 3, w.y() + w.h() - 3);
                        if (px != lightBg) return a + " still " + Integer.toHexString(px);
                    }
                    return null;
                }, "Terminal, Files, the Editor and Settings all have the light background", 10_000)
                .note("A permission prompt from a real app: in the Terminal, install Net Check from the sample packages and run it; Deny")
                .mutate(r -> clickText(r, "taskbar", "Terminal"), "click the Terminal's taskbar button")
                .await(r -> appWindow(r, "terminal") != null && appWindow(r, "terminal").focused() ? null : "Terminal not focused", "the Terminal has focus", 5_000);
        terminalCommand(b, "store --sample-repo /srv/r")
                .await(r -> shows(r, "terminal", "store: wrote the sample repository"), "the sample packages are written", 15_000);
        terminalCommand(b, "pkg install /srv/r/" + NETCHECK + "-1.0.0.ecmpkg")
                .await(r -> shows(r, "terminal", "Installed Net Check"), "Net Check is installed from its package file", 15_000);
        terminalCommand(b, "pkg run " + NETCHECK)
                .await(r -> comp(r).layerTexts("permission").contains(NETCHECK_ASKS) ? null : "no dialog yet",
                        "the desktop's dialog asks \"" + NETCHECK_ASKS + "\"", 30_000)
                .mutate(r -> snapshot(r, "gui_desktop_prompt"), "snapshot: the permission dialog")
                .mutate(r -> clickText(r, "permission", "Deny"), "click Deny")
                .await(r -> netCheck(r) != null && netCheck(r).denials().contains("network") ? null : "no denial recorded",
                        "the answer is deny (the app manager recorded it)", 10_000)
                .await(r -> comp(r).layerTexts("permission").isEmpty() ? null : "dialog still open", "the dialog is gone", 5_000)
                .waitMs(200, "a refresh, so the snapshot shows it gone")
                .mutate(r -> snapshot(r, "gui_desktop_final"), "snapshot: the desktop at the end")
                .mutate(r -> EvansComputerMod.LOGGER.info("[gui_desktop] {}", ui(r).status().strip()), "log the windows");
        return b.build();
    }

    // ================================================================ gui_client_render

    static com.example.evanscomputermod.testing.SceneViewerMirror viewer(ScenarioRun r) {
        return VIEWER.get(r);
    }

    /** Pump the client and compare its display with the compositor's frame. */
    static String clientMatches(ScenarioRun r) {
        var v = viewer(r);
        if (v == null) return "no viewer";
        v.pump();
        if (!v.active()) return "the client doesn't draw the GUI itself";
        return v.compare(comp(r));
    }

    /**
     * gui_client_render (UI_PROTOCOL.md §14 phase 2a): a player whose client
     * draws the GUI itself, played headless with the real hub, packet codec and
     * client code. Its display must equal the server's frame pixel for pixel;
     * typing in Notes must cost it under 1 KB/s; a legacy pixel window streams
     * as per-surface tiles. Controls: a player without the client capability,
     * client rendering switched off, and the console, all go back to tiles.
     * The watching client is the scenario's one stand-in (a GameTest server
     * has no client; the real client is checked by {@code -ClientChecks
     * -ClientSuite gui}); the typing and clicking are the scenario player's.
     */
    private static Scenario guiClientRender() {
        var b = Scenario.builder("gui_client_render",
                        "the GUI drawn by the client from scene patches: identical pixels, typing under 1 KB/s, fallbacks")
                .asPlayer()
                .host("pc", PC, "-")
                .decor(new Desk())
                .note("Setup (done for you): a Standard SDR placed east of the Terminal; arm.gba copied into the computer's folder"
                        + " (binary, docs/SCENARIO_EXCEPTIONS.md)")
                .note("Write /etc/ui.toml from Python: the test shell opens Notes");
        PlayerKit.typeFile(b, "pc", "/etc/ui.toml", UI_TOML_NOTES);
        return b.note("Start the GUI: the test shell opens Notes")
                .send("pc", "gui")
                .expect("pc", "^gui: started", "the compositor and the test shell started")
                .await(r -> comp(r) != null && comp(r).shellConnected() ? null : "waiting for the shell", "the test shell connected", 15_000)
                .await(r -> tiled(r, 1), "Notes' window drawn", 30_000)
                .note("A player whose client can draw scenes watches (it said hello with the scene stream version) [exception: a"
                        + " headless client, docs/SCENARIO_EXCEPTIONS.md]")
                .mutate(r -> VIEWER.put(r, new com.example.evanscomputermod.testing.SceneViewerMirror(ui(r), r.terminal("pc").getComputerId())),
                        "a scene-capable player opens the Terminal")
                .await(r -> viewer(r).eligible() ? null : "not streamed as a scene", "the server streams the GUI to it as a scene", 1_000)
                .await(r -> ui(r).sceneViewers(0, List.of(java.util.UUID.randomUUID())).isEmpty() ? null : "a client without the hello got scenes",
                        "control: a player whose client never said hello stays on tile deltas", 1_000)
                .await(GuiScenarios::clientMatches, "the client draws the desktop itself, identical to the server's frame", 10_000)
                .note("Type into Notes at about 100 words a minute: the patches cost the player under 1 KB/s")
                .mutate(r -> {
                    Compositor.WindowInfo w = mapped(r).stream().filter(x -> x.appId().equals("notes")).findFirst().orElseThrow();
                    // Notes' text field is its first row (12 lu padding): click into it.
                    click(r, new int[]{w.x() + 40, w.y() + 22});
                }, "click Notes' text field")
                .await(r -> {
                    viewer(r).pump();
                    return null;
                }, "let the click settle", 1_000)
                .waitMs(500, "the field takes focus")
                .await(r -> {
                    var v = viewer(r);
                    v.pump();
                    long now = System.currentTimeMillis();
                    long[] t = TYPING.computeIfAbsent(r, k -> new long[]{now, v.bytes(), 0, v.counters().steps()});
                    long due = t[0] + t[2] * 1000L / KEYS_PER_SECOND;
                    if (t[2] < KEYS && now >= due) {
                        type(r, String.valueOf("the quick brown fox jumps".charAt((int) t[2])));
                        t[2]++;
                    }
                    // A quiet tail after the last key, so its patch is counted.
                    return t[2] >= KEYS && now >= due + 250 ? null : "typing (" + t[2] + " keys)";
                }, KEYS + " keystrokes at " + KEYS_PER_SECOND + " a second", 10_000)
                .await(r -> {
                    var v = viewer(r);
                    long[] t = TYPING.get(r);
                    double seconds = (double) KEYS / KEYS_PER_SECOND;
                    long bytes = v.bytes() - t[1], steps = v.counters().steps() - t[3];
                    double perS = bytes / seconds;
                    EvansComputerMod.LOGGER.info("[gui_client_render] typing {} keys in Notes: {} scene steps, {} B, {} B/s",
                            KEYS, steps, bytes, Math.round(perS));
                    if (steps < KEYS / 2) return "only " + steps + " patches while typing: the keys didn't reach Notes";
                    return perS < 1024 ? null : "typing costs " + Math.round(perS) + " B/s";
                }, "typing in Notes cost the watching player under 1 KB/s (and did change the scene)", 1_000)
                .await(GuiScenarios::clientMatches, "after typing, the client still shows exactly the server's frame", 5_000)
                .note("A legacy pixel program in a window: Ctrl+Alt+Backspace, gui launch gba arm.gba, Ctrl+Alt+Backspace;"
                        + " its pixels stream as tiles of its own surface")
                .mutate(GuiScenarios::chord, "press Ctrl+Alt+Backspace (console)")
                .await(r -> !ui(r).guiVisible() ? null : "GUI still shown", "the console is shown", 5_000)
                .send("pc", "gui launch gba arm.gba")
                .expect("pc", "^gui: launched gba", "gba starts in a window")
                .mutate(GuiScenarios::chord, "press Ctrl+Alt+Backspace (GUI)")
                .await(r -> ui(r).guiVisible() ? null : "console still shown", "the GUI is shown", 5_000)
                .await(r -> tiled(r, 2), "Notes and gba tiled", 30_000)
                .await(r -> drawing(r, "gba"), "gba draws the test ROM", 30_000)
                .await(GuiScenarios::clientMatches, "the client shows gba's frames too, identical to the server's", 10_000)
                .note("Control: the operator sets ui.clientRendering = false in the server config: back to tile deltas")
                .mutate(r -> EcmConfig.setUiClientRendering(false), "serverconfig: ui.clientRendering = false")
                .await(r -> {
                    viewer(r).pump();
                    return viewer(r).active() ? "the client still draws scenes" : null;
                }, "the stream ended (END): the client hands the display back to tiles", 2_000)
                .mutate(r -> EcmConfig.setUiClientRendering(true), "serverconfig: ui.clientRendering = true")
                .await(GuiScenarios::clientMatches, "a fresh keyframe and the client draws it again, identical", 10_000)
                .note("Ctrl+Alt+Backspace: the console is pixels again, so the scene stream ends; back to the GUI, it resumes")
                .mutate(GuiScenarios::chord, "press Ctrl+Alt+Backspace")
                .await(r -> {
                    viewer(r).pump();
                    return !ui(r).guiVisible() && !viewer(r).active() ? null : "still streaming a scene";
                }, "console shown, scene stream ended", 5_000)
                .mutate(GuiScenarios::chord, "press Ctrl+Alt+Backspace again")
                .await(GuiScenarios::clientMatches, "the GUI again, drawn by the client, identical", 10_000)
                .mutate(r -> {
                    var v = viewer(r);
                    var c = v.counters();
                    EvansComputerMod.LOGGER.info("[gui_client_render] the player got {} B in {} messages ({} steps, {} keyframes, {} images)",
                            c.bytes(), c.messages(), c.steps(), c.keyframes(), c.resources());
                    v.pump();
                    v.close();
                }, "log what the player was sent")
                .timeLimit(60_000)
                .build();
    }

    /** The floor, the SDR (placed by the player) and the test ROM in the computer's folder. */
    private static final class Desk implements Scenario.Decor {
        private final List<BlockPos> floor = new ArrayList<>();

        Desk() {
            for (int x = -3; x <= 3; x++)
                for (int z = -2; z <= 2; z++) floor.add(new BlockPos(x, 0, z));
        }

        @Override
        public List<BlockPos> footprint() {
            List<BlockPos> all = new ArrayList<>(floor);
            all.add(SDR);
            return all;
        }

        @Override
        public void terrain(ScenarioRun run) {
            PlayerKit.fill(run, floor, Blocks.SMOOTH_STONE.defaultBlockState());
        }

        @Override
        public void build(ScenarioRun run) {
            run.player().place(RadioSdrContent.SDR_STANDARD.get(), run.abs(SDR), Direction.SOUTH,
                    s -> s.getValue(SdrBlock.FACING) == Direction.SOUTH);
            // Exception (docs/SCENARIO_EXCEPTIONS.md): a binary ROM can't be typed; players copy it into the folder.
            PlayerKit.dropFiles(run, "pc", "", ROM_DIR, List.of("arm.gba"));
        }

        @Override
        public void clear(ScenarioRun run) {
            UiManager u = ui(run);
            if (u != null) u.stop();
            try {
                EcmConfig.setUiClientRendering(true);   // undo gui_client_render's control if it stopped half way
            } catch (RuntimeException e) {
                // the server config isn't loaded (nothing to undo)
            }
        }
    }

    // Last: static initialisers run in source order, and the scenarios use the constants above.
    static {
        Scenario s = guiBasics();
        ALL.put(s.name, s);
        Scenario d = guiDesktop();
        ALL.put(d.name, d);
        Scenario c = guiClientRender();
        ALL.put(c.name, c);
    }
}
//?}
