package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.block.TerminalBlock;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.card.CardContent;
import com.example.evanscomputermod.card.CardReaderBlock;
import com.example.evanscomputermod.computer.WorldNetwork;
import com.example.evanscomputermod.computer.ui.Compositor;
import com.example.evanscomputermod.radio.cellular.CellularContent;
import com.example.evanscomputermod.radio.framed.PhyProfile;
import com.example.evanscomputermod.worldgen.TechVillageLocator;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.phys.Vec3;

import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.WeakHashMap;
import java.util.function.Function;

/**
 * The Browser ({@code dev.ecm.browser}, docs/gui/BROWSER.md), phase 1: spawnable with
 * {@code /ecm scenario spawn <name>} and run as GameTests in {@code ecm_browser}.
 *
 * <ul>
 *   <li>{@code browser_local_site}: a player writes a small site (index.html, style.css with a
 *       flexbox bar and cards, a second page) on computer A, opens it in the Browser as
 *       {@code /home/site/index.html} (file://), then serves the folder with {@code httpd 80}; on
 *       a second, cabled computer the Browser loads it over HTTP (the stylesheet too) and a link
 *       is followed. Controls: a page whose stylesheet is missing still shows, unstyled; a page
 *       the server doesn't have shows the server's 404.</li>
 *   <li>{@code browser_village}: a Tech Village house PC (its router and PC started by opening
 *       them); {@code gui}, the Browser from the launcher, the village site's address typed,
 *       a click on "Tech Village 2" loads the next village's site over the fiber ring. Controls:
 *       an address nobody answers shows an error page; {@code /nope.html} shows the server's 404.</li>
 *   <li>{@code browser_phone}: an ePhone on a player-built cell network (core, cell site, a web
 *       server on the core's LAN, as in {@code cell_chat}). Control first: no modem, no signal:
 *       the Browser shows an error page and stays up. With an LTE Modem and a SIM it attaches,
 *       Reload loads the page over LTE, a finger drag scrolls to its end, a tap follows a link.</li>
 * </ul>
 */
public final class BrowserScenarios {
    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();
    static final String APP = "dev.ecm.browser";
    static final String PROMPT = PlayerKit.PROMPT;
    /** The protocol's Ctrl modifier bit. */
    static final int CTRL = 0x2;
    static final int HID_A = 0x04, HID_ENTER = 0x28;

    private BrowserScenarios() {}

    // ================================================================ a computer the player works at

    /** A Terminal the player sits at: a scenario node, or one found in the world. */
    record Pc(String name, Function<ScenarioRun, TerminalBlockEntity> be) {
        static Pc node(String n) {
            return new Pc(n, r -> r.terminal(n));
        }

        Compositor comp(ScenarioRun r) {
            TerminalBlockEntity t = be.apply(r);
            if (t == null || t.getComputer() == null || t.getComputer().getUiManager() == null) return null;
            return t.getComputer().getUiManager().compositor();
        }

        Compositor.WindowInfo window(ScenarioRun r) {
            Compositor c = comp(r);
            if (c == null) return null;
            return c.windows().stream().filter(w -> !w.shell() && w.mapped() && w.placed() && w.visible() && w.appId().equals(APP))
                    .findFirst().orElse(null);
        }

        /** Null once the Browser's window shows a text containing {@code text}. */
        String shows(ScenarioRun r, String text) {
            Compositor.WindowInfo w = window(r);
            if (w == null) return "no Browser window on " + name;
            List<String> texts = comp(r).textsOf(w.id());
            return texts.stream().anyMatch(t -> t.contains(text)) ? null : "the Browser doesn't show '" + text + "': " + texts;
        }

        int[] locate(ScenarioRun r, String text) {
            Compositor.WindowInfo w = window(r);
            return w == null ? null : comp(r).locateText(w.id(), text, false);
        }

        void sit(ScenarioRun r) {
            r.player().sitAt(be.apply(r).getBlockPos());
        }

        java.util.UUID id(ScenarioRun r) {
            return be.apply(r).getComputerId();
        }

        void click(ScenarioRun r, int[] at) {
            sit(r);
            if (!r.player().click(id(r), IComputerHost.DISPLAY_OWN, at[0], at[1])) r.fail(name + " refused the player's click");
        }

        void clickText(ScenarioRun r, String text) {
            int[] at = locate(r, text);
            if (at == null) {
                r.fail("can't see '" + text + "' in the Browser on " + name);
                return;
            }
            r.say("§7  click '" + text + "' at " + at[0] + "," + at[1]);
            click(r, at);
        }

        void type(ScenarioRun r, String text) {
            sit(r);
            if (!r.player().type(id(r), IComputerHost.DISPLAY_OWN, text)) r.fail(name + " refused the player's typing");
        }

        void key(ScenarioRun r, int hid, int mods, String console) {
            sit(r);
            if (!r.player().press(id(r), IComputerHost.DISPLAY_OWN, hid, mods, console)) r.fail(name + " refused the player's key");
        }

        String screen(ScenarioRun r) {
            TerminalBlockEntity t = be.apply(r);
            return t == null ? "" : ScenarioRun.screen(t.getDisplay());
        }

        /** The composited colour (0xRRGGBB) at a device pixel. */
        int pixel(ScenarioRun r, int x, int y) {
            Compositor c = comp(r);
            byte[] f = c.frameRgba();
            int i = (y * c.display().deviceW() + x) * 4;
            return (f[i] & 0xFF) << 16 | (f[i + 1] & 0xFF) << 8 | (f[i + 2] & 0xFF);
        }

        /** The page's background: in the page's right margin, halfway down the window (clear of the scrollbar). */
        int pageBackground(ScenarioRun r) {
            Compositor.WindowInfo w = window(r);
            return pixel(r, w.x() + w.w() - 12, w.y() + w.h() / 2);
        }
    }

    /** The address bar: the topmost text in the window that is an address. */
    static int[] addressBar(Compositor c, Compositor.WindowInfo w) {
        if (c == null || w == null) return null;
        int[] best = null;
        for (String t : c.textsOf(w.id())) {
            if (!(t.startsWith("about:") || t.startsWith("http://") || t.startsWith("file://"))) continue;
            int[] at = c.locateText(w.id(), t, false);
            if (at != null && (best == null || at[1] < best[1])) best = at;
        }
        return best;
    }

    /** {@code gui} at the console; the desktop connects. */
    static void startGui(Scenario.Builder b, Pc pc) {
        b.mutate(r -> {
            pc.type(r, "gui");
            pc.key(r, HID_ENTER, 0, "\n");
        }, "type gui on " + pc.name + " (the desktop)");
        b.await(r -> pc.comp(r) != null && pc.comp(r).shellConnected() ? null : "no desktop yet: " + pc.screen(r).strip(),
                "the desktop is up on " + pc.name, 20_000);
    }

    /** The Browser from the taskbar's launcher: the Apps button, then Browser. */
    static void openBrowser(Scenario.Builder b, Pc pc) {
        b.await(r -> pc.comp(r) != null && pc.comp(r).locateLayerText("taskbar", "Apps", false) != null ? null : "no taskbar yet",
                "the taskbar's Apps button is drawn", 10_000);
        b.mutate(r -> pc.click(r, pc.comp(r).locateLayerText("taskbar", "Apps", false)), "click Apps on the taskbar");
        b.await(r -> pc.comp(r).locatePopupText("Browser", false) != null ? null : "the launcher has no Browser",
                "the launcher lists Browser (a system app)", 5_000);
        b.mutate(r -> pc.click(r, pc.comp(r).locatePopupText("Browser", false)), "click Browser");
        b.await(r -> pc.window(r) != null ? null : "no Browser window yet", "the Browser opens (its start page)", 20_000);
        b.await(r -> pc.shows(r, "Tech Village sites"), "the start page lists the Tech Village sites", 10_000);
    }

    /** Click the address bar, select its text (Ctrl+A), type an address, Enter. */
    static void address(Scenario.Builder b, Pc pc, String addr) {
        addressFn(b, pc, r -> addr, addr);
    }

    static int[] at(ScenarioRun r, Pc pc, String text) {
        int[] a = pc.locate(r, text);
        if (a == null) throw new IllegalStateException("'" + text + "' isn't drawn");
        return a;
    }

    // ================================================================ browser_local_site

    static final String INDEX_HTML = """
            <!DOCTYPE html>
            <html><head><title>Alex's Workshop</title><link rel="stylesheet" href="style.css"></head>
            <body>
            <nav class="bar"><b class="logo">Alex's Workshop</b><a href="index.html">Builds</a><a href="parts.html">Parts</a></nav>
            <main>
            <h1>Redstone and fiber</h1>
            <p class="lead">Builds and parts lists from the house by the river.</p>
            <div class="cards">
            <div class="card"><h3>Sorter</h3><p>Hoppers, comparators and patience.</p></div>
            <div class="card"><h3>Fiber tap</h3><p>A patch panel on the span to village 4.</p></div>
            <div class="card"><h3>Weather</h3><p>Lidar on the roof, numbers in Python.</p></div>
            </div>
            </main>
            </body></html>
            """;

    static final String STYLE_CSS = """
            body { margin: 0; background: #f4f1ea; color: #2b2b2b }
            .bar { display: flex; gap: 16px; align-items: center; padding: 8px 16px; background: #2f5f9e }
            .bar a { color: #ffffff; text-decoration: none }
            .logo { color: #ffd166; margin-right: auto }
            main { padding: 8px 16px }
            .cards { display: flex; flex-wrap: wrap; gap: 12px }
            .card { flex: 1 1 120px; background: #ffffff; border: 1px solid #d8d2c4; border-radius: 6px; padding: 8px 12px }
            .card h3 { margin: 0 0 4px 0; color: #2f5f9e }
            """;

    static final String PARTS_HTML = """
            <html><head><title>Parts</title><link rel="stylesheet" href="style.css"></head>
            <body><main><h1>Parts list</h1><ul><li>64 hoppers</li><li>12 comparators</li></ul>
            <p><a href="index.html">Back home</a></p></main></body></html>
            """;

    /** Control: a page whose stylesheet isn't there. */
    static final String PLAIN_HTML = """
            <html><head><title>Plain</title><link rel="stylesheet" href="missing.css"></head>
            <body><h1>No style here</h1><p>The stylesheet is missing, the page still shows.</p></body></html>
            """;

    static final String SITE_A = "10.0.60.1", SITE_B = "10.0.60.2";
    static final int PAGE_BG = 0xf4f1ea;

    static Scenario localSite() {
        Pc pc = Pc.node("pc"), viewer = Pc.node("viewer");
        var b = Scenario.builder("browser_local_site",
                        "a player writes a site with a flexbox stylesheet, previews it as a file in the Browser, serves it with httpd and"
                                + " loads it from a second computer over the cable; a link is followed")
                .asPlayer()
                .timeLimit(60_000);
        b.host("pc", new BlockPos(0, 1, 0), SITE_A + "/24");
        b.host("viewer", new BlockPos(4, 1, 0), SITE_B + "/24");
        b.link("lan", "pc", Direction.DOWN, "viewer", Direction.DOWN, Scenario.Path.from(new BlockPos(0, 0, 0)).go(Direction.EAST, 4));
        b.note("Setup (done for you): two computers, a cable between their bottom faces.");
        b.configureHosts();
        b.note("pc: write the site into /home/site (typed from Python, like edit would save it)");
        b.send("pc", "mkdir -p /home/site");
        b.expect("pc", PROMPT, "shell");
        PlayerKit.typeFiles(b, Map.of("pc", Map.of(
                "/home/site/index.html", INDEX_HTML,
                "/home/site/style.css", STYLE_CSS,
                "/home/site/parts.html", PARTS_HTML,
                "/home/site/plain.html", PLAIN_HTML)));
        b.note("pc: the desktop, the Browser, the site as a file");
        startGui(b, pc);
        openBrowser(b, pc);
        address(b, pc, "/home/site/index.html");
        b.await(r -> pc.shows(r, "Redstone and fiber"), "the page shows: file:///home/site/index.html", 15_000);
        b.await(r -> pc.shows(r, "file:///home/site/index.html"), "the address bar has the file address", 5_000);
        b.mutate(r -> VideoScenarios.snapshot(r, pc.comp(r), "browser_local_file"), "snapshot: the site as a file");
        b.await(r -> {
            int[] s = pc.locate(r, "Sorter"), f = pc.locate(r, "Fiber tap");
            int[] logo = pc.locate(r, "Alex's Workshop"), home = pc.locate(r, "Builds"), parts = pc.locate(r, "Parts");
            Compositor.WindowInfo w = pc.window(r);
            String where = "window " + w.w() + "x" + w.h() + ", texts " + pc.comp(r).textsOf(w.id());
            if (s == null || f == null || logo == null || home == null || parts == null) return "not all drawn: " + where;
            if (!(s[1] == f[1] && s[0] < f[0])) return "cards at " + s[0] + "," + s[1] + " / " + f[0] + "," + f[1] + "; " + where;
            if (!(logo[1] == home[1] && home[0] - logo[0] > 100 && parts[0] > home[0]))
                return "bar: logo " + logo[0] + " builds " + home[0] + " parts " + parts[0] + "; " + where;
            return null;
        }, "style.css applied: cards side by side (flex), Builds and Parts pushed right of the logo (margin-right: auto)", 5_000);
        b.await(r -> pc.pageBackground(r) == PAGE_BG ? null : String.format("page background %06x", pc.pageBackground(r)),
                "the page background is the stylesheet's #f4f1ea", 5_000);
        b.note("Control: a page whose stylesheet is missing still shows, unstyled");
        address(b, pc, "/home/site/plain.html");
        b.await(r -> pc.shows(r, "No style here"), "plain.html shows (missing.css didn't stop it)", 15_000);
        b.await(r -> pc.pageBackground(r) != PAGE_BG ? null : "the page has the site's background", "unstyled: the theme's background", 5_000);
        b.note("pc: back to the console (Ctrl+Alt+Backspace), serve the folder");
        b.mutate(r -> pc.key(r, ScenarioPlayer.HID_BACKSPACE, com.example.evanscomputermod.computer.ui.UiManager.SWITCH_MODS, null),
                "Ctrl+Alt+Backspace: the console");
        b.await(r -> pc.be.apply(r).getDisplay().getDisplayMode() == 0 ? null : "still the GUI", "the console is back", 5_000);
        b.send("pc", "httpd 80 /home/site &");
        b.expect("pc", "HTTP server listening on port 80, serving /home/site", "httpd serves /home/site");
        b.note("viewer: the desktop, the Browser, the site over HTTP");
        startGui(b, viewer);
        openBrowser(b, viewer);
        address(b, viewer, SITE_A);
        b.await(r -> viewer.shows(r, "Redstone and fiber"), "the page arrives from " + SITE_A, 15_000);
        b.expect("pc", "GET /style\\.css", "the viewer fetched the stylesheet from httpd");
        b.await(r -> viewer.pageBackground(r) == PAGE_BG ? null : String.format("page background %06x", viewer.pageBackground(r)),
                "styled over HTTP too", 5_000);
        b.mutate(r -> VideoScenarios.snapshot(r, viewer.comp(r), "browser_local_http"), "snapshot: the site over HTTP");
        b.mutate(r -> viewer.clickText(r, "Parts"), "click Parts");
        b.await(r -> viewer.shows(r, "Parts list"), "the link loads parts.html", 15_000);
        b.expect("pc", "GET /parts\\.html", "httpd served parts.html");
        b.await(r -> viewer.shows(r, "http://" + SITE_A + "/parts.html"), "the address bar follows the link", 5_000);
        b.note("Control: a page the server doesn't have");
        address(b, viewer, SITE_A + "/nope.html");
        b.await(r -> viewer.shows(r, "File not found"), "the server's 404 text shows", 15_000);
        b.expect("pc", "GET /nope\\.html", "httpd was asked for it");
        return b.build();
    }

    // ================================================================ browser_village

    /** Per run: the village and its house computers. */
    static final class Village {
        int n;
        TechVillageLocator.Visit visit;
        BlockPos pc, router;
    }

    private static final Map<ScenarioRun, Village> VILLAGES = Collections.synchronizedMap(new WeakHashMap<>());

    static Village vil(ScenarioRun r) {
        return VILLAGES.computeIfAbsent(r, k -> new Village());
    }

    static TerminalBlockEntity houseTerminal(ScenarioRun r, boolean pc) {
        Village v = vil(r);
        BlockPos at = pc ? v.pc : v.router;
        return at != null && r.level().getBlockEntity(at) instanceof TerminalBlockEntity t ? t : null;
    }

    /** The nearest Tech Village (placed at its planned site where this world has none, as in a GameTest) and the ring running. */
    static void village(ScenarioRun r) {
        var level = r.level();
        var d = WorldNetwork.get(level);
        d.plan(level);
        Village v = vil(r);
        v.n = d.nearest(r.origin()).number();
        try {
            v.visit = TechVillageLocator.visit(level, v.n);
        } catch (IllegalStateException e) {
            var start = TechVillageLocator.placeForTest(level, v.n);
            v.visit = TechVillageLocator.resolve(level, v.n, start);
            r.say("§7  village " + v.n + " placed at its planned site [exception: a GameTest world has structures off; docs/SCENARIO_EXCEPTIONS.md]");
        }
        TechVillageLocator.hold(level, v.visit.start(), true);
        // A normal server starts the ring's infrastructure at start (WorldNetwork.start); GameTest servers skip it.
        d.applyLinks(level);
        for (var site : d.villages)
            for (String role : d.infrastructure(site.number())) d.boot(level, site.number(), role);
        for (var t : v.visit.network().terminals()) {
            if (t.role().equals("house1.pc")) v.pc = t.pos();
            if (t.role().equals("house1.router")) v.router = t.pos();
        }
        if (v.pc == null || v.router == null) r.fail("village " + v.n + " has no house1 router/PC");
        else r.say("§7  village " + v.n + ": house PC at " + v.pc.toShortString() + ", its router at " + v.router.toShortString());
    }

    static Scenario village() {
        Pc pc = new Pc("the house PC", r -> houseTerminal(r, true));
        var b = Scenario.builder("browser_village",
                        "a Tech Village house PC: the Browser loads the village's website from its Data Center and follows a link to the"
                                + " next village's site over the fiber ring; an address nobody answers and a missing page are shown as such")
                .asPlayer()
                .inWorld()
                .timeLimit(60_000);
        b.note("Stand in a Tech Village (in a GameTest world the scenario places the nearest village at its planned site, and starts the ring's"
                + " infrastructure as a normal server does at start).");
        b.mutate(BrowserScenarios::village, "find the nearest Tech Village, its first house's router and PC; the ring is running");
        b.mutate(r -> {
            for (boolean isPc : new boolean[]{false, true}) {
                TerminalBlockEntity t = houseTerminal(r, isPc);
                if (t != null) r.player().openScreen(t.getBlockPos(), t.getBlockState().getValue(TerminalBlock.FACING));
            }
        }, "open the house router's screen, then the PC's (opening a computer starts it)");
        b.await(r -> pc.screen(r).contains("Welcome to Terminal OS") ? null : "booting: " + pc.screen(r).strip(), "the house PC has booted", 30_000);
        b.note("The house PC leases an address from its router and reaches the Data Center");
        long[] next = {0};
        b.await(r -> {
            String s = pc.screen(r);
            if (s.contains("1 packets sent, 1 received")) return null;
            int village = vil(r).n;
            if (System.currentTimeMillis() > next[0] && s.stripTrailing().endsWith(">")) {
                pc.type(r, "ping 100." + (64 + village) + ".0.10 -n 1");
                pc.key(r, HID_ENTER, 0, "\n");
                next[0] = System.currentTimeMillis() + 4000;
            }
            return "no reply yet:\n" + s.strip();
        }, "ping 100.(64+n).0.10 (the village web server) answers", 30_000);
        startGui(b, pc);
        openBrowser(b, pc);
        addressFn(b, pc, r -> "100." + (64 + vil(r).n) + ".0.10", "the village site's address, 100.(64+n).0.10");
        b.await(r -> {
            Compositor.WindowInfo w = pc.window(r);
            String want = "Tech Village " + vil(r).n + " - AS " + (65000 + vil(r).n);
            return w != null && w.title().startsWith(want) ? null : "window title " + (w == null ? "-" : w.title());
        }, "the village's website is shown (the window takes its title, Tech Village n - AS 6500n)", 15_000);
        b.await(r -> pc.shows(r, "on the Tech Village fiber ring"), "its text", 2_000);
        b.mutate(r -> VideoScenarios.snapshot(r, pc.comp(r), "browser_village_top"), "snapshot: the top of the village site");
        b.await(r -> {
            if (pc.shows(r, "Other villages") == null && pc.locate(r, "Tech Village " + (vil(r).n % 10 + 1)) != null) return null;
            Compositor.WindowInfo w = pc.window(r);
            pc.sit(r);
            if (!r.player().wheel(pc.id(r), IComputerHost.DISPLAY_OWN, w.x() + w.w() / 2, w.y() + w.h() / 2, -1))
                r.fail("the computer refused the wheel");
            return "scrolled one wheel step";
        }, "turn the mouse wheel until the list of the other villages is on screen", 15_000);
        b.mutate(r -> VideoScenarios.snapshot(r, pc.comp(r), "browser_village"), "snapshot: the village site");
        int[] other = {0};
        b.mutate(r -> {
            other[0] = vil(r).n % 10 + 1;
            pc.clickText(r, "Tech Village " + other[0]);
        }, "click the next village's link (Tech Village n+1)");
        b.await(r -> pc.shows(r, "http://100." + (64 + other[0]) + ".0.10/"), "the address bar has the next village's site", 10_000);
        b.await(r -> {
            Compositor.WindowInfo w = pc.window(r);
            String want = "Tech Village " + other[0] + " - AS " + (65000 + other[0]);
            return w != null && w.title().startsWith(want) ? null : "window title " + (w == null ? "-" : w.title());
        }, "the next village's site arrives over the fiber ring (the window takes its title)", 30_000);
        b.note("Control: an address nobody answers");
        addressFn(b, pc, r -> "100." + (64 + vil(r).n) + ".0.99", "100.(64+n).0.99");
        b.await(r -> pc.shows(r, "Can't reach this page"), "an error page: Can't reach this page", 30_000);
        b.note("Control: a page the web server doesn't have");
        addressFn(b, pc, r -> "100." + (64 + vil(r).n) + ".0.10/nope.html", "100.(64+n).0.10/nope.html");
        b.await(r -> pc.shows(r, "File not found"), "the server's 404 page", 15_000);
        return b.build();
    }

    static void addressFn(Scenario.Builder b, Pc pc, Function<ScenarioRun, String> addr, String shown) {
        b.mutate(r -> {
            int[] at = addressBar(pc.comp(r), pc.window(r));
            if (at == null) {
                r.fail("can't see the address bar on " + pc.name);
                return;
            }
            pc.click(r, at);
            pc.key(r, HID_A, CTRL, null);
            pc.type(r, addr.apply(r));
            pc.key(r, HID_ENTER, 0, "\n");
        }, "click the address bar, Ctrl+A, type " + shown + ", Enter");
    }

    // ================================================================ browser_phone

    static final String PH_CORE_IP = "10.0.72.1", PH_SITE_IP = "10.0.72.2", PH_LAN_IP = "10.0.73.1", PH_SERVER_IP = "10.0.73.10";
    static final String PH_POOL = "100.65.15.0/24", CARRIER = "cell-red";

    /** A long page (the phone scrolls to its end) with a link at the bottom. */
    static String newsPage() {
        StringBuilder s = new StringBuilder("<html><head><title>Village news</title></head><body><h1>Village news</h1>\n");
        for (int i = 1; i <= 40; i++) s.append("<p>Note ").append(i).append(": the mast is up.</p>\n");
        s.append("<p><a href=\"page2.html\">Older news</a></p></body></html>\n");
        return s.toString();
    }

    static final String PAGE2 = "<html><head><title>Older news</title></head><body><h1>Older news</h1><p>The ring was built.</p></body></html>\n";

    static Compositor.WindowInfo phoneWindow(ScenarioRun r) {
        return PhoneGuiScenarios.onScreen(r, APP);
    }

    static String phoneShows(ScenarioRun r, String text) {
        Compositor.WindowInfo w = phoneWindow(r);
        if (w == null) return "no Browser on screen: " + PhoneGuiScenarios.comp(r).windows();
        List<String> t = PhoneGuiScenarios.comp(r).textsOf(w.id());
        return t.stream().anyMatch(x -> x.contains(text)) ? null : "the Browser shows " + t;
    }

    static int[] phoneAt(ScenarioRun r, String text) {
        Compositor.WindowInfo w = phoneWindow(r);
        return w == null ? null : PhoneGuiScenarios.comp(r).locateText(w.id(), text, false);
    }

    static void phoneAddress(Scenario.Builder b, String addr) {
        b.mutate(r -> {
            int[] at = addressBar(PhoneGuiScenarios.comp(r), phoneWindow(r));
            if (at == null) {
                r.fail("no address bar on the phone");
                return;
            }
            PhoneGuiScenarios.tap(r, at);
            if (!r.player().chord(PhoneScenarios.st(r).phone, IComputerHost.DISPLAY_OWN, HID_A, CTRL)) r.fail("the phone refused Ctrl+A");
            PhoneGuiScenarios.type(r, addr);
            PhoneGuiScenarios.key(r, HID_ENTER);
        }, "tap the address bar, select it, type " + addr + ", Enter");
    }

    static Scenario phone() {
        List<BlockPos> floor = new ArrayList<>(PlayerKit.box(-2, 0, -8, 26, 0, 3));
        List<BlockPos> cable = Scenario.Path.from(new BlockPos(0, 0, -6)).go(Direction.SOUTH, 6).blocks();
        floor.removeAll(cable);
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(cable);
        foot.addAll(FramedScenarios.station(CellPhoneScenarios.SITE));
        foot.add(CellPhoneScenarios.CORE_READER);
        var b = Scenario.builder("browser_phone",
                        "an ePhone's Browser: with no signal an error page (no crash); on a cell network it loads a page over LTE, a"
                                + " finger drag scrolls it, a tap follows a link")
                .asPlayer()
                .realTime()
                .host("core", CellPhoneScenarios.CORE, "-")
                .host("site", CellPhoneScenarios.SITE, "-")
                .host("server", CellPhoneScenarios.SERVER, "-")
                .link("backhaul", "core", Direction.DOWN, "site", Direction.DOWN, Scenario.Path.from(new BlockPos(0, 0, -6)).go(Direction.SOUTH, 6))
                .link("lan", "core", Direction.UP, "server", Direction.UP, Scenario.Path.from(new BlockPos(0, 2, -6)).go(Direction.EAST, 4))
                .decor(PlayerKit.decor(foot, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()), r -> {
                    FramedScenarios.build(r, CellPhoneScenarios.SITE);
                    r.player().place(CardContent.CARD_READER.get(), r.abs(CellPhoneScenarios.CORE_READER), Direction.NORTH,
                            s -> s.getValue(CardReaderBlock.FACING) == Direction.NORTH);
                }, PhoneScenarios::cleanup))
                .note("Built for you (as in cell_chat): the core cabled (bottom) to the cell site's computer and (top) to a web server;"
                        + " a Card Reader/Writer against the core's east side; beside the site an Advanced SDR, coax and an LTE Sector"
                        + " Antenna pointing east.")
                .mutate(r -> {
                    PhoneScenarios.givePhone(r);
                    com.example.evanscomputermod.phone.PhoneConfig.override("battery.enabled", false);
                }, "the player takes a new ePhone (it boots while the network is built; batteries off for the run)")
                .mutate(CellPhoneScenarios::phoneToOffHand, "the phone into the off hand, the main hand free for building");
        b.send("server", "ifconfig eth1 " + PH_SERVER_IP + "/24");
        b.send("site", "ifconfig eth0 " + PH_SITE_IP + "/24");
        b.expect("server", "eth1: inet 10\\.0\\.73\\.10/24", "server has " + PH_SERVER_IP);
        b.send("server", "ip route add " + PH_POOL + " via " + PH_LAN_IP + " dev eth1");
        b.expect("server", "^Route 100\\.65\\.15\\.0/24 added", "the server reaches the phones through the core");
        b.note("server: a web page (typed from Python), httpd");
        b.send("server", "mkdir -p /srv/www");
        b.expect("server", PROMPT, "shell");
        PlayerKit.typeFiles(b, Map.of("server", Map.of("/srv/www/index.html", newsPage(), "/srv/www/page2.html", PAGE2)));
        b.send("server", "httpd 80 /srv/www &");
        b.expect("server", "HTTP server listening on port 80, serving /srv/www", "httpd serves /srv/www");
        b.expect("site", "eth0: inet 10\\.0\\.72\\.2/24", "site has " + PH_SITE_IP);
        CellularScenarios.routerCli(b, "core", "configure terminal", "interface eth0", "ip address " + PH_CORE_IP + "/24", "exit",
                "interface eth1", "ip address " + PH_LAN_IP + "/24", "exit", "ip routing", "end");
        CellularScenarios.coreUp(b, "core", CARRIER, PH_POOL, "site-a");
        CellularScenarios.siteUp(b, "site", "site-a", CARRIER, PH_CORE_IP, 1, PhyProfile.LTE, 1, "");
        b.note("core: a SIM (a blank card in the Card Reader/Writer, cored sim new, taken out by hand)");
        b.mutate(r -> r.player().use(new ItemStack(CellularContent.SIM_CARD.get()), r.abs(CellPhoneScenarios.CORE_READER), Direction.NORTH, false),
                "right-click the Card Reader/Writer with a blank SIM card");
        b.send("core", "cored sim new --label reader");
        b.expectOrFail("core", "^cored: wrote SIM IMSI 001\\d{12} \\(reader\\)", "cored writes the SIM", "^cored: (?!wrote)");
        b.mutate(r -> {
            r.player().useEmpty(r.abs(CellPhoneScenarios.CORE_READER), Direction.NORTH, false);
            if (CellPhoneScenarios.simSlot(r) < 0) {
                r.fail("taking the card out gave the player no written SIM: " + r.player().heard());
                return;
            }
            PhoneScenarios.holder(r).getInventory().setItem(21, new ItemStack(CellularContent.modem(PhyProfile.LTE)));
        }, "take the SIM out of the reader (empty hand); the player also has an LTE Modem");
        b.await(PhoneGuiScenarios::guiBooted, "the phone has booted into its shell", 40_000)
                .mutate(r -> PhoneScenarios.standAt(r.player(), r, new Vec3(20.5, 0, 0.5)), "walk 17 blocks east of the site's antenna")
                .mutate(CellPhoneScenarios::phoneToHand, "the phone in the main hand");
        b.note("Control: no modem yet, no signal. The Browser from the home grid; the server's address");
        PhoneGuiScenarios.openApp(b, "Browser", APP);
        b.await(r -> phoneShows(r, "Tech Village sites"), "the start page", 10_000);
        phoneAddress(b, PH_SERVER_IP);
        b.await(r -> phoneShows(r, "Can't reach this page") == null || phoneShows(r, "No network") == null ? null : phoneShows(r, "reach"),
                "an error page (no signal), not a crash", 30_000);
        b.await(r -> PhoneGuiScenarios.app(r, APP) != null ? null : "the Browser stopped", "the Browser is still running", 1_000);
        b.mutate(r -> snapshot(r, "browser_phone_no_signal"), "snapshot: the error page");
        b.note("The LTE Modem and the SIM in; the phone attaches");
        b.mutate(CellPhoneScenarios::fitModemAndSim, "slot screen: LTE Modem and SIM in")
                .await(CellPhoneScenarios::celldRunning, "celld runs", 15_000)
                .await(r -> CellPhoneScenarios.statusHas(r, "\ncarrier " + CARRIER + "\n"), "the status bar shows " + CARRIER + " (attached)", 20_000);
        b.await(r -> {
            int[] at = phoneAt(r, "Reload");
            if (at == null) return "no Reload button: " + phoneShows(r, "Reload");
            PhoneGuiScenarios.tap(r, at);
            return null;
        }, "tap Reload (the bottom bar)", 5_000);
        b.await(r -> phoneShows(r, "Village news"), "the page arrives over LTE", 30_000);
        b.expect("server", "^GET /\\s*$", "httpd served it");
        b.await(r -> phoneShows(r, "Older news") != null ? null : "the link at the end is already on screen",
                "the end of the page is off screen", 2_000);
        b.mutate(r -> snapshot(r, "browser_phone_page"), "snapshot: the page on the phone");
        b.note("Scroll with a finger: drags up the page until its last link shows");
        b.await(r -> {
            if (phoneShows(r, "Older news") == null) return null;
            Compositor.WindowInfo w = phoneWindow(r);
            int x = w.x() + w.w() / 2, y0 = w.y() + w.h() - 80, y1 = w.y() + 80;
            if (!r.player().drag(PhoneScenarios.st(r).phone, IComputerHost.DISPLAY_OWN, x, y0, x, y1, 8))
                r.fail("the phone refused the drag");
            return "dragged; not at the end yet";
        }, "drag up until the last link (Older news) is on screen", 15_000);
        b.await(r -> phoneShows(r, "Note 1:") != null ? null : "the top is still on screen", "the top of the page scrolled away", 2_000);
        b.await(r -> {
            int[] at = phoneAt(r, "Older news");
            if (at == null) return "no link";
            PhoneGuiScenarios.tap(r, at);
            return null;
        }, "tap Older news", 5_000);
        b.await(r -> phoneShows(r, "The ring was built."), "the link loads page2.html", 20_000);
        b.expect("server", "GET /page2\\.html", "httpd served page2.html");
        b.mutate(r -> snapshot(r, "browser_phone_link"), "snapshot: the followed link");
        b.timeLimit(60_000);
        return b.build();
    }

    static void snapshot(ScenarioRun r, String name) {
        VideoScenarios.snapshot(r, PhoneGuiScenarios.comp(r), name);
    }

    static {
        for (Scenario s : List.of(localSite(), village(), phone())) ALL.put(s.name, s);
    }
}
//?}
