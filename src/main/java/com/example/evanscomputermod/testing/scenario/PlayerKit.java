package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.ComputerStorage;
import com.example.evanscomputermod.module.ModuleBays;
import com.example.evanscomputermod.radio.wifi.RadioWifiContent;
import com.example.evanscomputermod.radio.wifi.WifiModule;
import com.example.evanscomputermod.radio.wifi.ap.ApPackets;
import com.example.evanscomputermod.radio.wifi.ap.ApSettings;
import com.example.evanscomputermod.radio.wifi80211.Security;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.entity.ChestBlockEntity;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.ChestBlock;

import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.List;
import java.util.function.Consumer;

/**
 * Building blocks for player-built radio scenarios ({@link Scenario.Builder#asPlayer}):
 * vanilla terrain (set like /fill), mod blocks and modules placed by the
 * {@link ScenarioPlayer}, Access Points configured through their screen's
 * handler, and the typed commands that write files and configure Wi-Fi.
 */
final class PlayerKit {
    /** A finished command: the shell prompt is back. */
    static final String PROMPT = "^/\\S* >$";

    private PlayerKit() {}

    // ------------------------------------------------------------ decor

    /** Decor from parts: vanilla terrain first, then what the player places, then cleanup. */
    static Scenario.Decor decor(List<BlockPos> footprint, Consumer<ScenarioRun> terrain, Consumer<ScenarioRun> build,
                                Consumer<ScenarioRun> clear) {
        List<BlockPos> f = List.copyOf(footprint);
        return new Scenario.Decor() {
            @Override
            public List<BlockPos> footprint() {
                return f;
            }

            @Override
            public void terrain(ScenarioRun run) {
                if (terrain != null) terrain.accept(run);
            }

            @Override
            public void build(ScenarioRun run) {
                if (build != null) build.accept(run);
            }

            @Override
            public void clear(ScenarioRun run) {
                if (clear != null) clear.accept(run);
            }
        };
    }

    /** Every position of the box (inclusive), relative. */
    static List<BlockPos> box(int x0, int y0, int z0, int x1, int y1, int z1) {
        List<BlockPos> out = new ArrayList<>();
        for (int x = Math.min(x0, x1); x <= Math.max(x0, x1); x++)
            for (int y = Math.min(y0, y1); y <= Math.max(y0, y1); y++)
                for (int z = Math.min(z0, z1); z <= Math.max(z0, z1); z++) out.add(new BlockPos(x, y, z));
        return out;
    }

    /** Set vanilla blocks (terrain: like /fill or a creative build). */
    static void fill(ScenarioRun run, List<BlockPos> rel, BlockState state) {
        for (BlockPos p : rel) run.level().setBlock(run.abs(p), state, 3);
    }

    /** The player installs an expansion card and a Wi-Fi Module in each named computer (right-clicks on its left side). */
    static void wifiModules(ScenarioRun run, String... nodes) {
        for (String n : nodes) {
            List<String> said = run.player().installModule(run.where(n), RadioWifiContent.WIFI_MODULE.get());
            TerminalBlockEntity t = run.terminal(n);
            boolean in = false;
            for (int i = 0; t != null && i < ModuleBays.SLOTS; i++)
                in |= t.getModuleBays().getStack(i).is(RadioWifiContent.WIFI_MODULE.get());
            if (!in)
                throw new IllegalStateException("Wi-Fi module not installed in " + n + ": the game said " + said);
        }
        run.say("§7  Wi-Fi modules clicked into " + String.join(", ", nodes) + " (expansion card first, left side)");
    }

    static WifiModule wifiModule(TerminalBlockEntity t) {
        if (t == null) return null;
        ModuleBays bays = t.getModuleBays();
        for (int i = 0; i < ModuleBays.SLOTS; i++) if (bays.getModule(i) instanceof WifiModule w) return w;
        return null;
    }

    /** The Wi-Fi module of {@code node} (fails when it has none). */
    static WifiModule moduleOf(ScenarioRun run, String node) {
        WifiModule w = wifiModule(run.terminal(node));
        if (w == null) throw new IllegalStateException("no Wi-Fi module in " + node);
        return w;
    }

    /** The player places an Access Point at {@code rel} and sets it up in its screen. */
    static void accessPoint(ScenarioRun run, BlockPos rel, Direction standOn, String ssid, String passphrase, int channel, int txDbm) {
        BlockPos p = run.abs(rel);
        run.player().place(com.example.evanscomputermod.radio.wifi.ap.AccessPointContent.ACCESS_POINT.get(), p, standOn);
        Security sec = passphrase == null ? Security.OPEN : Security.WPA2_PSK;
        String reply = run.player().configureAccessPoint(p, new ApSettings(ssid, false, sec, channel, txDbm, false, null, null), passphrase);
        if (!reply.equals("Settings applied")) throw new IllegalStateException("AP screen at " + p + " said: " + reply);
        run.say("§7  Access Point at " + p.toShortString() + ": SSID " + ssid + ", " + sec + ", channel " + channel + ", " + txDbm
                + " dBm (" + reply + ")");
    }

    /** What the AP screen's client list shows for {@code mac} (a row, or null). */
    static ApPackets.ClientRow apClient(ScenarioRun run, BlockPos rel, byte[] mac) {
        ApPackets.ApView v = run.player().accessPointScreen(run.abs(rel));
        if (v == null) return null;
        String want = com.example.evanscomputermod.radio.wifi80211.MacAddress.of(mac).toString();
        for (ApPackets.ClientRow r : v.clients()) if (r.mac().equalsIgnoreCase(want)) return r;
        return null;
    }

    /** Place a run of {@code block}s, each clicked onto the one before ({@code first} clicks its own neighbour). */
    static void placeRun(ScenarioRun run, Block block, List<BlockPos> rel, Direction standOn) {
        for (int i = 0; i < rel.size(); i++) {
            BlockPos p = run.abs(rel.get(i));
            if (i == 0) run.player().place(block, p, standOn);
            else {
                BlockPos prev = run.abs(rel.get(i - 1));
                Direction against = null;
                for (Direction d : Direction.values()) if (p.relative(d).equals(prev)) against = d;
                run.player().placeAgainst(block, p, standOn, against, null);
            }
        }
    }

    /** A vanilla chest at {@code rel} holding {@code items} (like a creative player filling one). */
    static void chest(ScenarioRun run, BlockPos rel, Direction facing, ItemStack... items) {
        BlockPos p = run.abs(rel);
        run.level().setBlock(p, Blocks.CHEST.defaultBlockState().setValue(ChestBlock.FACING, facing), 3);
        if (run.level().getBlockEntity(p) instanceof ChestBlockEntity c)
            for (int i = 0; i < items.length; i++) c.setItem(i, items[i]);
    }

    static void emptyChest(ScenarioRun run, BlockPos rel) {
        if (run.level().getBlockEntity(run.abs(rel)) instanceof ChestBlockEntity c) c.clearContent();
    }

    /**
     * Copy bundled files into a computer's storage folder, the way a player
     * drops files into {@code <world>/computer-data/<computer id>/<dir>/}.
     */
    static Path dropFiles(ScenarioRun run, String node, String dir, String resourceDir, List<String> names) {
        TerminalBlockEntity t = run.terminal(node);
        if (t == null) throw new IllegalStateException("no computer " + node);
        Path to = ComputerStorage.path(t).resolve(dir);
        try {
            Files.createDirectories(to);
            for (String n : names) {
                try (InputStream in = PlayerKit.class.getResourceAsStream(resourceDir + n)) {
                    if (in == null) throw new IllegalStateException("missing bundled file " + resourceDir + n);
                    Files.copy(in, to.resolve(n), StandardCopyOption.REPLACE_EXISTING);
                }
            }
        } catch (IOException e) {
            throw new IllegalStateException("copying files into " + to, e);
        }
        return to;
    }

    // ------------------------------------------------------------ typed recipes

    /** The Python REPL's prompt, waiting for the next line. */
    static final String PY_PROMPT = "^>>>$";
    static final String PY_ERROR = "Traceback|Error:";

    /** Start the Python REPL on {@code node}. */
    static void pythonStart(Scenario.Builder b, String node) {
        b.send(node, "python");
        b.expect(node, PY_PROMPT, node + ": Python REPL");
    }

    private static int tags;

    /**
     * Type a statement into the REPL, followed by {@code print("<tag>")} so the
     * scenario can see it ran (the REPL doesn't echo what is typed into it).
     */
    static void py(Scenario.Builder b, String node, String stmt, String what) {
        String tag = "ok" + (++tags);
        b.send(node, stmt + "; print(\"" + tag + "\")");
        b.await(r -> pyOutput(r, node, tag, ""), what, 20_000);
    }

    /** Type {@code print("<tag>", expr)}; done when the screen shows the tag followed by {@code re}. */
    static void pyPrint(Scenario.Builder b, String node, String expr, String re, String what) {
        String tag = "ok" + (++tags);
        b.send(node, "print(\"" + tag + "\", " + expr + ")");
        b.await(r -> pyOutput(r, node, tag, " " + re), what, 20_000);
    }

    /** {@link #pyPrint} with the expression worked out when the step runs ({@code shown} goes in the walkthrough). */
    static void pyPrintFn(Scenario.Builder b, String node, java.util.function.Function<ScenarioRun, String> expr, String shown,
                          String re, String what) {
        String tag = "ok" + (++tags);
        b.sendFn(node, r -> "print(\"" + tag + "\", " + expr.apply(r) + ")", "print(\"" + tag + "\", " + shown + ")");
        b.await(r -> pyOutput(r, node, tag, " " + re), what, 20_000);
    }

    /** Null once {@code node}'s screen shows the printed {@code tag} (not inside the typed code) followed by {@code after}. */
    static String pyOutput(ScenarioRun r, String node, String tag, String after) {
        String scr = r.screen(node);
        if (java.util.regex.Pattern.compile("(?<![\"\\w])" + tag + after).matcher(scr).find()) return null;
        java.util.regex.Matcher err = java.util.regex.Pattern.compile("(Traceback|\\w+Error: .*)").matcher(scr);
        if (err.find()) return "python says: " + err.group(1);
        return "waiting for " + tag;
    }

    static void pythonEnd(Scenario.Builder b, String node) {
        b.send(node, "exit()");
        b.expect(node, PROMPT, node + ": back at the shell");
    }

    /** Per run and node: where {@link #scan} is in its type-scan / type-results cycle. */
    private static final java.util.Map<String, int[]> SCANS = new java.util.concurrent.ConcurrentHashMap<>();

    /**
     * An Await check: type {@code iw dev wlan0 scan}, then {@code wpa_cli scan_results}
     * (one line per network), and repeat until the results match {@code want};
     * fails the run at once if they ever match {@code never}. {@code rounds}
     * &gt; 0 instead stops after that many result lists (the control: never seen).
     */
    static java.util.function.Function<ScenarioRun, String> scan(String node, String want, String never, int rounds) {
        java.util.regex.Pattern w = want == null ? null : java.util.regex.Pattern.compile(want, java.util.regex.Pattern.MULTILINE);
        java.util.regex.Pattern n = never == null ? null : java.util.regex.Pattern.compile(never, java.util.regex.Pattern.MULTILINE);
        java.util.regex.Pattern prompt = java.util.regex.Pattern.compile(PROMPT, java.util.regex.Pattern.MULTILINE);
        String key = node + "@" + System.identityHashCode(w) + "/" + System.identityHashCode(n);
        return r -> {
            int[] st = SCANS.computeIfAbsent(System.identityHashCode(r) + key, k -> new int[2]);
            String out;
            switch (st[0]) {
                case 0 -> { r.typeLine(node, "iw dev wlan0 scan"); st[0] = 1; return "scanning"; }
                case 1 -> {
                    out = r.latestOutput(node);
                    if (out == null || !prompt.matcher(out).find()) return "scanning";
                    r.typeLine(node, "wpa_cli scan_results");
                    st[0] = 2;
                    return "reading the scan results";
                }
                default -> {
                    out = r.latestOutput(node);
                    if (out == null || !prompt.matcher(out).find()) return "reading the scan results";
                    st[0] = 0;
                    st[1]++;
                    if (n != null && n.matcher(out).find()) {
                        r.fail("scan " + st[1] + " on " + node + " lists what it must not (" + never + "):\n" + out.strip());
                        return "failed";
                    }
                    if (w != null && w.matcher(out).find() || rounds > 0 && st[1] >= rounds) {
                        SCANS.remove(System.identityHashCode(r) + key);
                        return null;
                    }
                    return "scan " + st[1] + " results:\n" + out.strip();
                }
            }
        };
    }

    /**
     * Write a text file on {@code node} from the Python REPL ({@code shell.write_file}),
     * the way a player without an editor would; {@code lines} are joined with newlines.
     */
    static void typeFile(Scenario.Builder b, String node, String path, String... lines) {
        StringBuilder text = new StringBuilder();
        for (String l : lines) text.append(l.replace("\\", "\\\\").replace("\"", "\\\"")).append("\\n");
        int slash = path.lastIndexOf('/');
        if (slash > 0) {
            b.send(node, "mkdir -p " + path.substring(0, slash));
            b.expect(node, PROMPT, path.substring(0, slash) + " exists");
        }
        pythonStart(b, node);
        py(b, node, "import shell", "import shell");
        pyPrint(b, node, "shell.write_file(\"" + path + "\", \"" + text + "\")", "True", path + " written");
        pythonEnd(b, node);
    }

    /** Join a WPA2 network on wlan0 with wpa_cli + wpa_supplicant (static address first). */
    static void joinWpa2(Scenario.Builder b, String node, String ip, String ssid, String psk) {
        b.send(node, "ifconfig wlan0 " + ip);
        b.expect(node, "wlan0: inet " + java.util.regex.Pattern.quote(ip), node + " has " + ip + " on wlan0");
        b.send(node, "wpa_cli add_network");
        b.expect(node, "^0$", node + ": network 0 added");
        b.send(node, "wpa_cli set_network 0 ssid " + ssid);
        b.expect(node, "^OK$", node + ": ssid set", "^FAIL");
        b.send(node, "wpa_cli set_network 0 psk \"" + psk + "\"");
        b.expect(node, "^OK$", node + ": passphrase set", "^FAIL");
        b.send(node, "wpa_cli enable_network 0");
        b.expect(node, "^OK$", node + ": network 0 enabled", "^FAIL");
        b.await(scan(node, "\\s" + java.util.regex.Pattern.quote(ssid) + "$", null, 0),
                node + ": iw dev wlan0 scan, then wpa_cli scan_results, until it lists " + ssid, 20_000);
        b.send(node, "wpa_supplicant -B -D packet -i wlan0 -c /etc/wpa_supplicant.conf");
        b.expect(node, PROMPT, node + ": wpa_supplicant runs in the background", "wpa_supplicant:");
    }
}
//?}
