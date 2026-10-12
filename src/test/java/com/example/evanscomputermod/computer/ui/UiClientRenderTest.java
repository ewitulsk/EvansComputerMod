package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.DisplaySyncHub;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.ui.proto.UiCodec;
import com.example.evanscomputermod.computer.ui.proto.UiProto;
import com.example.evanscomputermod.network.ClientDisplayCache;
import com.example.evanscomputermod.network.TerminalDeltaPacket;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.DynamicTest;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.TestFactory;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assertions.fail;

/**
 * Client-side rendering, phase 2a (ROADMAP P8 "Done when", UI_PROTOCOL.md
 * §14/§17), without a game:
 * <ul>
 *   <li>every golden session replayed through the server compositor, the
 *       scene stream (encoded, compressed and decoded) and a client
 *       {@link UiSceneMirror} gives the server's frame and the checked-in
 *       PNG pixel for pixel, both for a client following every step from
 *       the start (its own damage tracking, client-side transitions) and
 *       for a late joiner starting from a keyframe;</li>
 *   <li>typing in an editor-like app costs a watching player under 1 KB/s;</li>
 *   <li>a Tier 3 display showing the test shell desktop costs less as a
 *       scene than a Tier 2 display does as tile deltas.</li>
 * </ul>
 */
public class UiClientRenderTest {

    @BeforeEach
    void clean() {
        UiClients.clear();
        ClientUiScenes.clearAll();
    }

    // ================================================================ determinism

    @TestFactory
    List<DynamicTest> goldenSessionsRenderTheSameOnTheClient() {
        List<DynamicTest> out = new ArrayList<>();
        for (String s : UiGoldenTest.SESSIONS) out.add(DynamicTest.dynamicTest(s, () -> clientMatchesServer(s)));
        return out;
    }

    /** A client applying every step as it comes, checked at every snapshot (plus a late joiner there). */
    private void clientMatchesServer(String session) throws Exception {
        Map<Long, int[]> images = new HashMap<>();
        UiSceneMirror mirror = new UiSceneMirror(images::get);
        List<SceneExporter.Step> steps = new ArrayList<>();
        SceneExporter[] ex = new SceneExporter[1];
        int[] snapshots = {0}, applied = {0};
        List<String> failures = new ArrayList<>();
        UiGoldenTest.replay(session, new UiGoldenTest.Hook() {
            @Override
            public void started(Compositor comp, UiTestHost host) {
                ex[0] = new SceneExporter(comp, 41, steps::add);
                comp.setExporter(ex[0]);
                // The client joins at once: a keyframe of the (empty) scene, then every step.
                SceneExporter.Step kf = ex[0].keyframe();
                steps.clear();
                deliver(kf, mirror, images, ex[0], host.now);
            }

            @Override
            public void ticked(Compositor comp, UiTestHost host) {
                for (SceneExporter.Step s : steps) {
                    deliver(s, mirror, images, ex[0], host.now);
                    applied[0]++;
                }
                steps.clear();
                mirror.render(host.now);
            }

            @Override
            public void snapshot(String name, Compositor comp, UiTestHost host) throws Exception {
                snapshots[0]++;
                byte[] server = comp.frameRgba();
                UiDisplay d = comp.display();
                String why = diff(name + " (client following the stream)", server, mirror.rgba(), d.deviceW());
                if (why != null) failures.add(why);
                String golden = UiGoldenTest.compare(name, mirror.rgba().clone(), d.deviceW(), d.deviceH());
                if (golden != null) failures.add("client: " + golden);
                // A late joiner: one keyframe, drawn whole.
                Map<Long, int[]> imgs2 = new HashMap<>();
                UiSceneMirror late = new UiSceneMirror(imgs2::get);
                SceneExporter.Step kf = ex[0].keyframe();
                for (SceneExporter.Step s : steps) deliver(s, mirror, images, ex[0], host.now);
                steps.clear();
                deliver(kf, late, imgs2, ex[0], host.now);
                late.renderAll(host.now);
                why = diff(name + " (late joiner)", server, late.rgba(), d.deviceW());
                if (why != null) failures.add(why);
            }
        });
        assertTrue(snapshots[0] > 0, session + ": no snapshots");
        assertTrue(applied[0] > 0, session + ": no steps were applied");
        if (!failures.isEmpty()) fail(String.join("\n", failures));
        System.out.println("CLIENT_RENDER " + session + ": " + snapshots[0] + " snapshots identical, " + applied[0]
                + " steps, " + ex[0].steps() + " built (" + ex[0].stepBytes() + " bytes), " + ex[0].keyframes()
                + " keyframes (" + ex[0].keyframeBytes() + " bytes)");
    }

    /** What a client does with a message: images it needs first, then the patch. */
    static void deliver(SceneExporter.Step s, UiSceneMirror mirror, Map<Long, int[]> images, SceneExporter ex, long now) {
        try {
            if (s.images() != null) {
                for (long k : s.images()) {
                    if (images.containsKey(k)) continue;
                    SceneExporter.Image img = ex.image(k);
                    assertNotNull(img, "image " + Long.toHexString(k) + " gone");
                    // Through the wire format, like the real RESOURCE message.
                    byte[] data = UiSceneCodec.pack(UiSceneCodec.RESOURCE, s.epoch(),
                            UiSceneCodec.encodeResource(k, img.w(), img.h(), img.argb()), new java.util.zip.Deflater());
                    UiSceneCodec.Resource r = UiSceneCodec.unpack(data).resource();
                    images.put(r.key(), r.argb());
                    mirror.imageArrived(r.key(), r.argb(), now);
                }
            }
            UiSceneMirror.Result res = mirror.apply(UiSceneCodec.unpack(s.data()), now);
            assertEquals(UiSceneMirror.Result.APPLIED, res, "step " + s.from() + "->" + s.to() + (s.keyframe() ? " (keyframe)" : ""));
            assertTrue(mirror.missing().isEmpty(), "missing images " + mirror.missing());
        } catch (com.example.evanscomputermod.computer.ui.proto.UiProtoException e) {
            throw new AssertionError(e);
        }
    }

    static String diff(String what, byte[] server, byte[] client, int w) {
        if (client == null) return what + ": the client drew nothing";
        if (server.length != client.length) return what + ": frame sizes differ";
        int n = 0, first = -1;
        for (int i = 0; i < server.length; i += 4) {
            if (server[i] != client[i] || server[i + 1] != client[i + 1] || server[i + 2] != client[i + 2]) {
                if (n++ == 0) first = i / 4;
            }
        }
        return n == 0 ? null : what + ": " + n + " pixels differ, first at " + (first % w) + "," + (first / w);
    }

    // ================================================================ a scripted editor

    static byte[] enc(UiProto.Message... ms) {
        return UiCodec.encodeAll(List.of(ms));
    }

    static UiConnection open(Compositor c, int pid, UiProto.Role role) {
        int[] err = new int[1];
        UiConnection conn = c.open(pid, role, err);
        assertNotNull(conn, "ui_open: " + err[0]);
        assertEquals(0, conn.send(enc(new UiProto.Hello(UiProto.VERSION, role, List.of()))));
        return conn;
    }

    static <T extends UiProto.Message> List<T> got(UiConnection c, Class<T> type) throws Exception {
        List<T> out = new ArrayList<>();
        for (UiProto.Message m : UiGoldenTest.drain(c)) if (type.isInstance(m)) out.add(type.cast(m));
        return out;
    }

    static final int LINES = 22, LINE_H = 16, COLS = 76;

    /**
     * An editor-like app: a full-display window with a background, a title
     * bar and {@link #LINES} lines of mono text, plus a caret. A tiny shell
     * places it full screen.
     */
    static final class Editor {
        final UiConnection app, shell;
        final StringBuilder[] lines = new StringBuilder[LINES];
        int line;

        Editor(Compositor comp, int w, int h) throws Exception {
            shell = open(comp, 1, UiProto.Role.SHELL);
            app = open(comp, 2, UiProto.Role.APP);
            assertEquals(0, app.send(enc(new UiProto.SurfaceCreate(1, UiProto.SurfaceKind.TREE, UiProto.SurfaceRole.WINDOW,
                    UiProto.Layer.NONE, 0, 0, 0, 0, 0, UiProto.PixelFormat.NONE, 1, "editor"))));
            List<UiProto.WindowOpened> opened = got(shell, UiProto.WindowOpened.class);
            assertEquals(1, opened.size());
            assertEquals(0, shell.send(enc(new UiProto.Place(opened.get(0).window(), 0, 0, w, h, 0,
                    new UiProto.Transition(0, UiProto.Easing.LINEAR)), new UiProto.FocusWindow(opened.get(0).window()))));
            List<UiProto.Configure> cfg = got(app, UiProto.Configure.class);
            assertEquals(1, cfg.size());
            List<UiProto.TreeOp> ops = new ArrayList<>();
            ops.add(rect(1, 0, 0, w, h, 0xF4F1EAFF));
            ops.add(rect(2, 0, 0, w, 20, 0x2B3A55FF));
            ops.add(text(3, 6, 2, "notes.txt - Editor", UiProto.Font.UI, 16, 0xFFFFFFFF));
            for (int i = 0; i < LINES; i++) {
                lines[i] = new StringBuilder();
                ops.add(text(10 + i, 6, 24 + i * LINE_H, "", UiProto.Font.MONO, 16, 0x202020FF));
            }
            ops.add(rect(5, 6, 24, 2, LINE_H, 0x2B3A55FF));
            assertEquals(0, app.send(enc(new UiProto.TreeOps(1, ops), new UiProto.AckConfigure(1, cfg.get(0).serial()),
                    new UiProto.Commit(1))));
        }

        static UiProto.TreeOp rect(int id, int x, int y, int w, int h, int fill) {
            return new UiProto.TreeOp.Create(id, 0, UiCodec.APPEND, UiProto.NodeKind.RECT,
                    List.of(new UiProto.Prop.X(x), new UiProto.Prop.Y(y), new UiProto.Prop.W(w), new UiProto.Prop.H(h),
                            new UiProto.Prop.Fill(fill)));
        }

        static UiProto.TreeOp text(int id, int x, int y, String s, UiProto.Font font, int size, int color) {
            List<UiProto.Prop> p = new ArrayList<>(List.of(new UiProto.Prop.X(x), new UiProto.Prop.Y(y)));
            if (!s.isEmpty()) p.add(new UiProto.Prop.Text(s));
            p.add(new UiProto.Prop.Font(font));
            p.add(new UiProto.Prop.Size(size));
            p.add(new UiProto.Prop.Color(color));
            p.sort(java.util.Comparator.comparingInt(UiProto.Prop::tag));
            return new UiProto.TreeOp.Create(id, 0, UiCodec.APPEND, UiProto.NodeKind.TEXT, p);
        }

        /** One keystroke: the line's text and the caret move, in one commit (what an editor sends). */
        void type(char ch) {
            if (ch == '\n' || lines[line].length() >= COLS) {
                line = (line + 1) % LINES;
                lines[line].setLength(0);
            }
            if (ch != '\n') lines[line].append(ch);
            int caretX = 6 + lines[line].length() * 8;
            assertEquals(0, app.send(enc(new UiProto.TreeOps(1, List.of(
                    new UiProto.TreeOp.Set(10 + line, List.of(new UiProto.Prop.Text(lines[line].toString()))),
                    new UiProto.TreeOp.Set(5, List.of(new UiProto.Prop.X(caretX), new UiProto.Prop.Y(24 + line * LINE_H))))),
                    new UiProto.Commit(1))));
        }
    }

    /** One watching player of a scene: a client ({@link ClientUiScenes}) that acks every message. */
    static final class SceneViewer {
        final UUID player = UUID.randomUUID(), computer = UUID.randomUUID();
        final int displayId;
        final UiSceneHub hub = new UiSceneHub();
        final SceneExporter exporter;
        final UiTestHost host;

        SceneViewer(Compositor comp, UiTestHost host, int displayId) {
            this.host = host;
            this.displayId = displayId;
            UiClients.hello(player, UiSceneCodec.VERSION, new long[0]);
            exporter = new SceneExporter(comp, 5, hub::offer);
            comp.setExporter(exporter);
        }

        /** A sync pass (what the server thread does after each composited frame); the client applies and acks. */
        void pass() {
            List<byte[]> got = new ArrayList<>();
            hub.sync(displayId, List.of(player), () -> exporter, (p, d, data) -> got.add(data));
            for (byte[] data : got) {
                ClientUiScenes.Ack ack = ClientUiScenes.onMessage(computer, displayId, data, host.now);
                if (ack != null) hub.onAck(player, displayId, ack.epoch(), ack.generation(), ack.missing());
            }
            ClientUiScenes.tick(host.now);
        }

        long bytes() {
            return hub.viewer(player, displayId).bytes();
        }

        UiSceneMirror mirror() {
            return ClientUiScenes.get(computer, displayId);
        }
    }

    /** One watching player getting tile deltas (the path before client rendering), acking every packet. */
    static final class TileViewer {
        final UUID player = UUID.randomUUID(), computer = UUID.randomUUID();
        final DisplaySyncHub hub = new DisplaySyncHub(0);
        final TerminalDisplay server = new TerminalDisplay(1, 1), client = new TerminalDisplay(1, 1);
        boolean hasKeyframe;

        /** Scan the composited damage into the server display, as UiManager.present does. */
        void present(UiTestHost host, int w, int h, byte[] rgba, List<Box> damage) {
            UiManager.scanOut(server, rgba, w, h, damage);
        }

        void pass() {
            List<TerminalDeltaPacket> got = new ArrayList<>();
            hub.sync(computer, server, List.of(player), (id, p) -> got.add(p));
            for (TerminalDeltaPacket p : got) {
                TerminalDeltaPacket.ParsedDelta d = p.parse();
                if (d.packetType() == TerminalDeltaPacket.PACKET_TYPE_KEYFRAME) {
                    com.example.evanscomputermod.network.DeltaApplier.applyKeyframe(client, d);
                    hasKeyframe = true;
                } else if (hasKeyframe) {
                    com.example.evanscomputermod.network.DeltaApplier.applyDelta(client, d);
                }
                hub.onClientReady(player, d.generation());
            }
        }

        long bytes() {
            return hub.bytesSent(player);
        }
    }

    // ================================================================ bandwidth

    @Test
    void typingInAnEditorSendsUnderOneKilobytePerSecond() throws Exception {
        UiTestHost host = new UiTestHost();
        TileViewer tiles = new TileViewer();
        host.onPresent = (rgba, w, h, damage) -> tiles.present(host, w, h, rgba, damage);
        Compositor comp = new Compositor(host, UiDisplay.terminal(), UiLimits.defaults(), 30, "", List.of(), "ecm-default");
        comp.setVisible(true);
        SceneViewer scene = new SceneViewer(comp, host, 0);
        Editor ed = new Editor(comp, comp.display().logicalW(), comp.display().logicalH());
        host.now += 100;
        comp.tick();
        scene.pass();
        tiles.pass();
        assertNotNull(scene.mirror(), "the client has a mirror");
        assertTrue(scene.mirror().ready(), "the client has the scene");
        long sceneStart = scene.bytes(), tileStart = tiles.bytes();

        // 10 keystrokes a second for 20 seconds, a new line every 40 characters.
        String text = "The quick brown fox jumps over the lazy dog. Pack my box with five dozen liquor jugs! ";
        int seconds = 20, perSecond = 10;
        for (int i = 0; i < seconds * perSecond; i++) {
            char ch = (i % 40 == 39) ? '\n' : text.charAt(i % text.length());
            ed.type(ch);
            host.now += 1000 / perSecond;
            comp.tick();
            scene.pass();
            tiles.pass();
        }
        long sceneBytes = scene.bytes() - sceneStart, tileBytes = tiles.bytes() - tileStart;
        double scenePerS = sceneBytes / (double) seconds, tilePerS = tileBytes / (double) seconds;
        System.out.printf("CLIENT_RENDER typing: scene %d B over %d s = %.0f B/s; tiles %d B = %.0f B/s (%.1fx)%n",
                sceneBytes, seconds, scenePerS, tileBytes, tilePerS, tilePerS / Math.max(1, scenePerS));
        assertTrue(scenePerS < 1024, "typing costs " + scenePerS + " B/s as a scene");
        // The client drew what the server drew.
        scene.mirror().renderAll(host.now);
        assertNull(diff("editor", comp.frameRgba(), scene.mirror().rgba(), comp.display().deviceW()));
        // And its display (what the Terminal screen and the wall draw) holds the same pixels.
        TerminalDisplay td = ClientDisplayCache.entry(scene.computer, 0).display;
        assertTrue(ClientDisplayCache.entry(scene.computer, 0).sceneDriven);
        assertNull(diff("client display", comp.frameRgba(), td.getPixelData(), comp.display().deviceW()));
        assertNull(diff("tile client", comp.frameRgba(), tiles.client.getPixelData(), comp.display().deviceW()));
    }

    @Test
    void tier3WallShowingTheDesktopCostsLessThanTier2Tiles() throws Exception {
        // Tier 2, tile deltas (what every wall cost before client rendering).
        long[] tier2Tiles = {0};
        TileViewer t2 = new TileViewer();
        UiGoldenTest.replay("tiled_shell", new UiGoldenTest.Hook() {
            @Override
            public void started(Compositor comp, UiTestHost host) {
                host.onPresent = (rgba, w, h, damage) -> t2.present(host, w, h, rgba, damage);
            }

            @Override
            public void ticked(Compositor comp, UiTestHost host) {
                t2.pass();
            }
        });
        tier2Tiles[0] = t2.bytes();

        // Tier 3 (1280x800 at scale 2: the same 640x400 lu layout), as tiles and as a scene.
        TileViewer t3 = new TileViewer();
        SceneViewer[] s3 = new SceneViewer[1];
        Compositor tier3 = UiGoldenTest.replay("tiled_shell", new UiGoldenTest.Hook() {
            @Override
            public UiDisplay display(int w, int h, int scale) {
                return new UiDisplay(w * 2, h * 2, scale * 2, 3, UiProto.Device.TERMINAL);
            }

            @Override
            public boolean goldens() {
                return false;
            }

            @Override
            public int pointerScale() {
                return 2;
            }

            @Override
            public void started(Compositor comp, UiTestHost host) {
                host.onPresent = (rgba, w, h, damage) -> t3.present(host, w, h, rgba, damage);
                s3[0] = new SceneViewer(comp, host, 1);
            }

            @Override
            public void ticked(Compositor comp, UiTestHost host) {
                t3.pass();
                s3[0].pass();
            }
        });
        long scene = s3[0].bytes();
        System.out.printf("CLIENT_RENDER desktop (tiled_shell): Tier 2 tiles %d B; Tier 3 tiles %d B; Tier 3 scene %d B (%.1fx less than Tier 2 tiles)%n",
                tier2Tiles[0], t3.bytes(), scene, tier2Tiles[0] / (double) Math.max(1, scene));
        assertTrue(scene < tier2Tiles[0], "Tier 3 scene " + scene + " B vs Tier 2 tiles " + tier2Tiles[0] + " B");
        // The Tier 3 client matches the Tier 3 server frame.
        s3[0].mirror().renderAll(s3[0].host.now);
        assertNull(diff("tier 3 wall", tier3.frameRgba(), s3[0].mirror().rgba(), tier3.display().deviceW()));
    }

    // ================================================================ stream behaviour

    @Test
    void transitionsAnimateOnTheClientWithNoTraffic() throws Exception {
        UiTestHost host = new UiTestHost();
        Compositor comp = new Compositor(host, UiDisplay.terminal(), UiLimits.defaults(), 30, "", List.of(), "ecm-default");
        comp.setVisible(true);
        SceneViewer v = new SceneViewer(comp, host, 0);
        Editor ed = new Editor(comp, 320, 200);
        host.now += 50;
        comp.tick();
        v.pass();
        long before = v.bytes();
        // The shell slides the window over 400 ms.
        assertEquals(0, ed.shell.send(enc(new UiProto.Place(1, 300, 180, 320, 200, 0,
                new UiProto.Transition(400, UiProto.Easing.EASE_IN_OUT)))));
        List<UiProto.Configure> cfg = got(ed.app, UiProto.Configure.class);
        assertTrue(cfg.isEmpty(), "same size: no configure");
        host.now += 1;
        comp.tick();
        v.pass();
        long afterStart = v.bytes();
        for (int i = 0; i < 12; i++) {
            host.now += 33;
            comp.tick();
            v.pass();
            String why = diff("frame " + i + " of the slide", comp.frameRgba(), v.mirror().rgba(), comp.display().deviceW());
            assertNull(why);
        }
        long during = v.bytes() - afterStart;
        System.out.printf("CLIENT_RENDER transition: start %d B, %d B during the 400 ms slide%n", afterStart - before, during);
        assertEquals(0, during, "nothing is sent while the client animates");
        host.now += 200;
        comp.tick();
        v.pass();
        assertNull(diff("after the slide", comp.frameRgba(), v.mirror().rgba(), comp.display().deviceW()));
    }

    @Test
    void lateJoinerFallingBehindAndEnd() throws Exception {
        UiTestHost host = new UiTestHost();
        Compositor comp = new Compositor(host, UiDisplay.terminal(), UiLimits.defaults(), 30, "", List.of(), "ecm-default");
        comp.setVisible(true);
        SceneViewer v = new SceneViewer(comp, host, 0);
        Editor ed = new Editor(comp, 640, 400);
        host.now += 50;
        comp.tick();
        v.pass();
        assertEquals(1, v.hub.viewer(v.player, 0).keyframes());
        // A client that stops acking keeps WINDOW steps in flight, then waits...
        List<byte[]> held = new ArrayList<>();
        for (int i = 0; i < UiSceneHub.WINDOW + 10; i++) {
            ed.type('x');
            host.now += 40;
            comp.tick();
            v.hub.sync(0, List.of(v.player), () -> v.exporter, (p, d, data) -> held.add(data));
        }
        assertEquals(UiSceneHub.WINDOW, held.size(), "steps in flight stop at the window");
        // ...and catches up from the history when its acks arrive: no keyframe.
        for (byte[] data : held) {
            ClientUiScenes.Ack a = ClientUiScenes.onMessage(v.computer, 0, data, host.now);
            v.hub.onAck(v.player, 0, a.epoch(), a.generation(), a.missing());
        }
        v.pass();
        assertEquals(1, v.hub.viewer(v.player, 0).keyframes(), "caught up from history");
        v.mirror().renderAll(host.now);
        assertNull(diff("caught up", comp.frameRgba(), v.mirror().rgba(), 640));
        // A second player joining now gets one keyframe and the same picture.
        UUID p2 = UUID.randomUUID(), c2 = v.computer;
        UiClients.hello(p2, UiSceneCodec.VERSION, new long[0]);
        List<byte[]> got2 = new ArrayList<>();
        v.hub.sync(1, List.of(p2), () -> v.exporter, (p, d, data) -> got2.add(data));
        assertEquals(1, got2.size());
        for (byte[] data : got2) ClientUiScenes.onMessage(c2, 1, data, host.now);
        ClientUiScenes.tick(host.now);
        assertNull(diff("late joiner", comp.frameRgba(), ClientUiScenes.get(c2, 1).rgba(), 640));
        // The display leaves GUI rendering: END, and the client gives the display back to tiles.
        List<byte[]> end = new ArrayList<>();
        v.hub.sync(0, List.of(), () -> v.exporter, (p, d, data) -> end.add(data));
        assertEquals(1, end.size());
        assertEquals(UiSceneCodec.END, end.get(0)[0]);
        ClientUiScenes.onMessage(v.computer, 0, end.get(0), host.now);
        assertNull(ClientUiScenes.get(v.computer, 0));
        assertTrue(!ClientDisplayCache.entry(v.computer, 0).sceneDriven);
        assertTrue(v.hub.isViewer(p2, 1), "the other display keeps its viewer");
    }

    @Test
    void imagesGoOncePerClient() throws Exception {
        UiTestHost host = new UiTestHost();
        Compositor comp = new Compositor(host, UiDisplay.terminal(), UiLimits.defaults(), 30, "", List.of(), "ecm-default");
        comp.setVisible(true);
        SceneViewer v = new SceneViewer(comp, host, 0);
        Editor ed = new Editor(comp, 640, 400);
        // A 32x32 image shown twice.
        byte[] rgba = new byte[32 * 32 * 4];
        for (int i = 0; i < 32 * 32; i++) {
            rgba[i * 4] = (byte) (i * 7);
            rgba[i * 4 + 1] = (byte) (i * 3);
            rgba[i * 4 + 2] = (byte) 200;
            rgba[i * 4 + 3] = (byte) 255;
        }
        assertEquals(0, ed.app.send(enc(new UiProto.ResourceCreate(9, UiProto.ResourceKind.IMAGE, 32, 32, UiProto.PixelFormat.NONE))));
        assertEquals(0, ed.app.upload(9, 1, rgba));
        assertEquals(0, ed.app.send(enc(new UiProto.TreeOps(1, List.of(image(50, 100, 100), image(51, 200, 100))),
                new UiProto.Commit(1))));
        host.now += 50;
        comp.tick();
        v.pass();
        assertEquals(1, v.hub.viewer(v.player, 0).resources(), "one image, sent once");
        v.mirror().renderAll(host.now);
        assertNull(diff("images", comp.frameRgba(), v.mirror().rgba(), 640));
        // A second display of the same computer for the same player: the client has it already.
        List<byte[]> got = new ArrayList<>();
        v.hub.sync(1, List.of(v.player), () -> v.exporter, (p, d, data) -> got.add(data));
        assertEquals(1, got.size(), "just the keyframe, no image data");
        // A client that reports it lost the image gets it again with a keyframe.
        v.hub.onAck(v.player, 0, 5, 0, new long[]{UiSceneCodec.imageKey(UiResource.fnv64(rgba), 32, 32)});
        v.pass();
        assertEquals(2, v.hub.viewer(v.player, 0).resources());
    }

    static UiProto.TreeOp image(int id, int x, int y) {
        return new UiProto.TreeOp.Create(id, 0, UiCodec.APPEND, UiProto.NodeKind.IMAGE,
                List.of(new UiProto.Prop.X(x), new UiProto.Prop.Y(y), new UiProto.Prop.W(64), new UiProto.Prop.H(64),
                        new UiProto.Prop.Resource(9)));
    }
}
