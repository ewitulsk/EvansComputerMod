package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.ui.proto.UiCodec;
import com.example.evanscomputermod.computer.ui.proto.UiProto;
import com.example.evanscomputermod.computer.wasi.WasiFunctions;
import com.example.evanscomputermod.network.ClientDisplayCache;
import com.example.evanscomputermod.network.ClientPlayout;
import com.example.evanscomputermod.network.ClockSync;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.BeforeEach;
import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.List;
import java.util.UUID;

import static org.junit.jupiter.api.Assertions.*;

/**
 * UI protocol version 2 on the scene path (DISPLAY_PIPELINE_SPEC.md Parts C3,
 * C4, C5, D, F), without a game: an app's canvas frames committed for a time
 * ({@code CommitAt}) go to a client ahead, are shown on the server and the
 * client at their time, are dropped by {@code QueueFlush}, may travel as JPEG
 * with an exact refine, and version 1 apps are unchanged. Then the client's
 * own feedback: hover styles, a scroll container, a window drag and typing.
 * Messages go through the real scene hub, codec and client mirror.
 */
public class UiTimedSceneTest {
    static final int CW = 64, CH = 64;
    final UiTestHost host = new UiTestHost();
    final UUID computer = UUID.randomUUID(), viewer = UUID.randomUUID();
    final UiSceneHub hub = new UiSceneHub();
    Compositor comp;
    SceneExporter exporter;

    @BeforeEach
    void setUp() {
        UiClients.clear();
        ClientUiScenes.clearAll();
        ClientDisplayCache.clear();
        ClientPlayout.clear();
        ClientPlayout.fixedDelayMs = 0;
        comp = new Compositor(host, UiDisplay.terminal(), UiLimits.defaults(), 30, "test-shell", List.of(), "ecm-default");
        comp.setVisible(true);
        exporter = new SceneExporter(comp, 77, hub::offer);
        exporter.setTimedSink(hub::offerTimed);
        comp.setExporter(exporter);
        UiClients.hello(viewer, UiSceneCodec.VERSION, new long[0]);
    }

    @AfterEach
    void tearDown() {
        ClientPlayout.fixedDelayMs = -1;
    }

    // ------------------------------------------------------------ a client

    /** One server pass and the client's handling of it (acks included, like the decode thread's receipt acks). */
    int pass() {
        int n = 0;
        for (int round = 0; round < 8; round++) {
            List<byte[]> sent = new ArrayList<>();
            hub.sync(0, List.of(viewer), () -> exporter, (p, d, data) -> sent.add(data));
            if (sent.isEmpty()) break;
            for (byte[] data : sent) {
                UiSceneCodec.Message m;
                try {
                    m = UiSceneCodec.unpack(data);
                } catch (Exception e) {
                    throw new AssertionError(e);
                }
                if (m.timed() != null) {
                    if (m.kind() == UiSceneCodec.TIMED) assertTrue(m.timed().decode(), "the timed frame decodes");
                    hub.onAck(viewer, 0, m.epoch(), UiSceneHub.TIMED_ACK | m.timed().tseq(), null, ClockSync.clientNowUs());
                }
                ClientUiScenes.Ack a = ClientUiScenes.onMessage(computer, 0, m, data.length, host.now, ClockSync.clientNowUs());
                if (a != null) hub.onAck(viewer, 0, a.epoch(), a.generation(), a.missing());
                n++;
            }
        }
        ClientUiScenes.tick(host.now);
        return n;
    }

    UiSceneMirror mirror() {
        return ClientUiScenes.get(computer, 0);
    }

    TerminalDisplay clientDisplay() {
        return ClientDisplayCache.entry(computer, 0).display;
    }

    /** The client's pixel at device (x, y), as 0xRRGGBB. */
    int clientPixel(int x, int y) {
        ClientUiScenes.tick(host.now);
        TerminalDisplay d = clientDisplay();
        byte[] px = d.getPixelData();
        int i = (y * d.getGfxWidth() + x) * 4;
        return (px[i] & 0xFF) << 16 | (px[i + 1] & 0xFF) << 8 | (px[i + 2] & 0xFF);
    }

    int serverPixel(int x, int y) {
        byte[] f = comp.frameRgba();
        int i = (y * comp.display().deviceW() + x) * 4;
        return (f[i] & 0xFF) << 16 | (f[i + 1] & 0xFF) << 8 | (f[i + 2] & 0xFF);
    }

    void tick() {
        host.now += 33;
        comp.tick();
    }

    static void sleep(long ms) {
        try {
            Thread.sleep(ms);
        } catch (InterruptedException e) {
            throw new AssertionError(e);
        }
    }

    // ------------------------------------------------------------ an app

    UiConnection open(int pid, int version) throws Exception {
        return open(pid, version, UiProto.Role.APP);
    }

    UiConnection open(int pid, int version, UiProto.Role role) throws Exception {
        int[] err = new int[1];
        UiConnection c = comp.open(pid, role, err);
        assertNotNull(c);
        assertEquals(0, c.send(UiCodec.encodeAll(List.of(new UiProto.Hello(version, role, List.of())))));
        return c;
    }

    /** A v2 app with one window showing canvas 1 at (0, 0), placed full-display by a shell. */
    UiConnection canvasApp() throws Exception {
        UiConnection shell = open(10, 2, UiProto.Role.SHELL);
        UiConnection app = open(11, 2);
        List<UiProto.Message> hello = UiGoldenTest.drain(app);
        assertTrue(hello.stream().anyMatch(m -> m instanceof UiProto.QueueLimits q && q.maxQueueFrames() == 12 && q.maxQueueMs() == 500),
                "a version 2 app is told the queue limits: " + hello);
        send(app, new UiProto.ResourceCreate(1, UiProto.ResourceKind.CANVAS, CW, CH, UiProto.PixelFormat.RGBA8888),
                new UiProto.SurfaceCreate(1, UiProto.SurfaceKind.TREE, UiProto.SurfaceRole.WINDOW, UiProto.Layer.NONE, 0, 0, 0, 0, 0,
                        UiProto.PixelFormat.NONE, 1, "video"));
        shellPlace(shell);
        int serial = UiCompositorTest.got(app, UiProto.Configure.class).get(0).serial();
        send(app, new UiProto.TreeOps(1, List.of(new UiProto.TreeOp.Create(1, 0, UiCodec.APPEND, UiProto.NodeKind.CANVAS,
                        List.of(new UiProto.Prop.W(CW), new UiProto.Prop.H(CH), new UiProto.Prop.Resource(1))))),
                new UiProto.AckConfigure(1, serial), new UiProto.Commit(1));
        put(app, 0x101010);
        send(app, new UiProto.Commit(1));
        tick();
        return app;
    }

    void shellPlace(UiConnection shell) throws Exception {
        assertEquals(0, shell.send(UiCodec.encodeAll(List.of(new UiProto.Place(1, 0, 0, 320, 200, 0,
                new UiProto.Transition(0, UiProto.Easing.LINEAR))))));
    }

    static void send(UiConnection c, UiProto.Message... ms) {
        assertEquals(0, c.send(UiCodec.encodeAll(List.of(ms))), "rejected: " + c.closeReason());
    }

    /** Canvas 1 filled with 0xRRGGBB (plus a gradient so frames differ in every tile when asked). */
    static void put(UiConnection app, int rgb) {
        byte[] px = new byte[CW * CH * 4];
        for (int i = 0; i < CW * CH; i++) {
            px[i * 4] = (byte) (rgb >> 16);
            px[i * 4 + 1] = (byte) (rgb >> 8);
            px[i * 4 + 2] = (byte) rgb;
            px[i * 4 + 3] = (byte) 0xFF;
        }
        assertEquals(0, app.putPixels(1, px.length, new int[0], (off, dst, dstOff, len) -> System.arraycopy(px, off, dst, dstOff, len)));
    }

    static long monoIn(long ms) {
        return WasiFunctions.monotonicNowUs() + ms * 1000;
    }

    // ------------------------------------------------------------ CommitAt

    @Test
    void canvasFramesCommittedForATimeGoAheadAndShowThen() throws Exception {
        UiConnection app = canvasApp();
        pass();
        assertTrue(mirror() != null && mirror().ready(), "the client draws the scene");
        assertEquals(0x101010, clientPixel(5, 5));
        put(app, 0x00FF00);
        send(app, new UiProto.CommitAt(1, monoIn(250)));
        put(app, 0x0000FF);
        send(app, new UiProto.CommitAt(1, monoIn(300)));
        tick();
        pass();
        assertEquals(2, mirror().timedQueued(), "the client holds both frames before their time");
        assertEquals(0x101010, serverPixel(5, 5), "the server shows the old frame until the time comes");
        assertEquals(0x101010, clientPixel(5, 5));
        sleep(350);
        tick(); // the pump timer showed the frames; the tick composites
        assertEquals(0x0000FF, serverPixel(5, 5), "the server shows the newest due frame");
        assertEquals(0x0000FF, clientPixel(5, 5), "so does the client, at its time");
        long[] t = hub.timedCounters();
        assertEquals(2, t[0], "each frame built once");
        assertTrue(comp.timedCounters()[1] >= 1, "shown on the server");
    }

    @Test
    void aFlushDropsQueuedFramesAndLayoutNeverWaits() throws Exception {
        UiConnection app = canvasApp();
        pass();
        for (int i = 0; i < 3; i++) {
            put(app, 0x200000 * (i + 1));
            send(app, new UiProto.CommitAt(1, monoIn(300 + i * 30)));
        }
        pass();
        assertEquals(3, mirror().timedQueued());
        send(app, new UiProto.QueueFlush(1));
        pass();
        assertEquals(0, mirror().timedQueued(), "the client dropped what it held");
        sleep(450);
        tick();
        pass();
        assertEquals(0x101010, serverPixel(5, 5), "no dropped frame ever shows on the server");
        assertEquals(0x101010, clientPixel(5, 5), "nor on the client");
        // Tree changes don't ride a CommitAt.
        send(app, new UiProto.TreeOps(1, List.of(new UiProto.TreeOp.Set(1, List.of(new UiProto.Prop.X(4))))));
        app.send(UiCodec.encodeAll(List.of(new UiProto.CommitAt(1, monoIn(100)))));
        assertTrue(app.isClosed(), "tree changes in a CommitAt are an error");
        assertEquals(UiProto.ErrorCode.BAD_STATE, app.closeReason().code);
    }

    @Test
    void aTimedProducerGetsFrameWhenTheQueueHasRoom() throws Exception {
        UiConnection app = canvasApp();
        UiGoldenTest.drain(app);
        put(app, 0x111111);
        send(app, new UiProto.CommitAt(1, monoIn(200)), new UiProto.RequestFrame(1));
        tick();
        assertEquals(1, UiCompositorTest.got(app, UiProto.Frame.class).size(), "room in the queue: Frame at once");
        for (int i = 0; i < 12; i++) {
            put(app, 0x111111 + i);
            send(app, new UiProto.CommitAt(1, monoIn(300)));
        }
        send(app, new UiProto.RequestFrame(1));
        tick();
        assertEquals(0, UiCompositorTest.got(app, UiProto.Frame.class).size(), "a full queue: no Frame yet");
        sleep(350);
        tick();
        assertEquals(1, UiCompositorTest.got(app, UiProto.Frame.class).size(), "the frames were shown: Frame again");
    }

    @Test
    void motionCanvasesGoAsJpegAndAreRefinedExactWhenTheyStop() throws Exception {
        UiConnection app = canvasApp();
        comp.display(); // Tier 2 terminal
        send(app, new UiProto.SurfaceSetContent(1, 1, 2));
        pass();
        for (int i = 0; i < 4; i++) {
            byte[] px = new byte[CW * CH * 4];
            for (int k = 0; k < CW * CH; k++) {
                px[k * 4] = (byte) (k * 3 + i * 40);
                px[k * 4 + 1] = (byte) (k / CW * 8);
                px[k * 4 + 2] = (byte) (i * 60);
                px[k * 4 + 3] = (byte) 0xFF;
            }
            assertEquals(0, app.putPixels(1, px.length, new int[0], (off, dst, dstOff, len) -> System.arraycopy(px, off, dst, dstOff, len)));
            send(app, new UiProto.CommitAt(1, monoIn(60 + i * 20)));
        }
        pass();
        assertTrue(hub.timedCounters()[1] >= 3, "motion frames are JPEG: " + java.util.Arrays.toString(hub.timedCounters()));
        assertTrue(hub.lossyOut());
        sleep(UiSceneHub.REFINE_AFTER_US / 1000 + 200);
        tick();
        pass();
        assertEquals(1, hub.timedCounters()[3], "one exact refine after the picture stood still");
        tick(); // the exporter's next step carries the canvas whole and exact
        pass();
        assertFalse(hub.lossyOut());
        ClientUiScenes.tick(host.now);
        for (int[] p : new int[][]{{1, 1}, {7, 20}, {30, 30}}) {
            assertEquals(serverPixel(p[0], p[1]), clientPixel(p[0], p[1]), "the still picture is exact at " + p[0] + "," + p[1]);
        }
    }

    @Test
    void versionOneAppsAreUnchanged() throws Exception {
        UiConnection v1 = open(20, 1);
        List<UiProto.Message> hello = UiGoldenTest.drain(v1);
        assertTrue(hello.get(0) instanceof UiProto.Welcome w && w.version() == 1, "Welcome answers with the app's version");
        assertFalse(hello.stream().anyMatch(m -> m instanceof UiProto.QueueLimits), "no v2 messages for a v1 app");
        send(v1, new UiProto.SurfaceCreate(1, UiProto.SurfaceKind.TREE, UiProto.SurfaceRole.WINDOW, UiProto.Layer.NONE, 0, 0, 0, 0, 0,
                UiProto.PixelFormat.NONE, 1, "old"));
        v1.send(UiCodec.encodeAll(List.of(new UiProto.CommitAt(1, 5))));
        assertTrue(v1.isClosed(), "CommitAt needs version 2");
        assertEquals(UiProto.ErrorCode.BAD_STATE, v1.closeReason().code);
    }

    // ------------------------------------------------------------ client feedback (Part F)

    /** A v2 app window with a hover button at (10..60, 10..30), a scroll list and a text field. */
    UiConnection feedbackApp() throws Exception {
        UiConnection shell = open(10, 2, UiProto.Role.SHELL);
        UiConnection app = open(11, 2);
        UiGoldenTest.drain(app);
        send(app, new UiProto.SurfaceCreate(1, UiProto.SurfaceKind.TREE, UiProto.SurfaceRole.WINDOW, UiProto.Layer.NONE, 0, 0, 0, 0, 0,
                UiProto.PixelFormat.NONE, 1, "feedback"));
        shellPlace(shell);
        int serial = UiCompositorTest.got(app, UiProto.Configure.class).get(0).serial();
        List<UiProto.TreeOp> ops = List.of(
                new UiProto.TreeOp.Create(1, 0, UiCodec.APPEND, UiProto.NodeKind.RECT, List.of(new UiProto.Prop.X(10), new UiProto.Prop.Y(10),
                        new UiProto.Prop.W(50), new UiProto.Prop.H(20), new UiProto.Prop.HitId(7), new UiProto.Prop.Fill(0x0000FFFF),
                        new UiProto.Prop.HoverFill(0xFF0000FF), new UiProto.Prop.PressedFill(0x00FF00FF))),
                new UiProto.TreeOp.Create(2, 0, UiCodec.APPEND, UiProto.NodeKind.GROUP, List.of(new UiProto.Prop.X(100), new UiProto.Prop.Y(10),
                        new UiProto.Prop.W(60), new UiProto.Prop.H(50), new UiProto.Prop.Clip(true), new UiProto.Prop.Scroll(2),
                        new UiProto.Prop.ContentH(200))),
                new UiProto.TreeOp.Create(3, 2, UiCodec.APPEND, UiProto.NodeKind.RECT, List.of(new UiProto.Prop.W(60),
                        new UiProto.Prop.H(40), new UiProto.Prop.Fill(0xFFFF00FF))),
                new UiProto.TreeOp.Create(4, 2, UiCodec.APPEND, UiProto.NodeKind.RECT, List.of(new UiProto.Prop.Y(40),
                        new UiProto.Prop.W(60), new UiProto.Prop.H(160), new UiProto.Prop.Fill(0x00FFFFFF))));
        send(app, new UiProto.TreeOps(1, ops), new UiProto.AckConfigure(1, serial), new UiProto.Commit(1));
        tick();
        pass();
        return app;
    }

    @Test
    void hoverAndPressedStylesShowAtOnceOnTheClient() throws Exception {
        feedbackApp();
        int sc = comp.display().scale();
        int bx = 20 * sc, by = 15 * sc;
        assertEquals(0x0000FF, clientPixel(bx, by));
        assertNull(ClientUiScenes.pointer(computer, 0, 1, bx, by, 0, 0, host.now));
        assertEquals(0xFF0000, clientPixel(bx, by), "hovered: the client shows the hover fill at once");
        assertEquals(0x0000FF, serverPixel(bx, by), "the server's drawing is unchanged (it is the app's state)");
        ClientUiScenes.pointer(computer, 0, 2, bx, by, 0, 0, host.now);
        assertEquals(0x00FF00, clientPixel(bx, by), "pressed");
        ClientUiScenes.pointer(computer, 0, 3, bx, by, 0, 0, host.now);
        ClientUiScenes.pointer(computer, 0, 1, 5 * sc, 5 * sc, 0, 0, host.now);
        assertEquals(0x0000FF, clientPixel(bx, by), "the pointer left: the app's own fill again");
        assertTrue(mirror().predictions() >= 0);
    }

    @Test
    void aScrollContainerScrollsOnTheClientAndTheAppConfirmsIt() throws Exception {
        UiConnection app = feedbackApp();
        int sc = comp.display().scale();
        int lx = 120 * sc, ly = 40 * sc; // content y 30: yellow; one step (16 lu) down it is content y 46: cyan
        assertEquals(0xFFFF00, clientPixel(lx, ly), "the top row of the list");
        ClientPrediction.ScrollReport r = ClientUiScenes.pointer(computer, 0, 4, lx, ly, 0, -1, host.now);
        assertNotNull(r, "a wheel over a scroll container is used on the client");
        assertEquals(ClientPrediction.WHEEL_STEP_LU, r.y());
        assertEquals(0x00FFFF, clientPixel(lx, ly), "the content moved up at once");
        assertEquals(0xFFFF00, serverPixel(lx, ly), "the server waits for the app");
        // The report reaches the app as Scrolled; it lays the content out there and confirms with ScrollY.
        comp.scrolled(r.surface(), r.node(), r.x(), r.y());
        List<UiProto.Scrolled> got = UiCompositorTest.got(app, UiProto.Scrolled.class);
        assertEquals(1, got.size());
        assertEquals(2, got.get(0).node());
        assertEquals(r.y(), got.get(0).y());
        send(app, new UiProto.TreeOps(1, List.of(new UiProto.TreeOp.Set(2, List.of(new UiProto.Prop.ScrollY(r.y()))),
                new UiProto.TreeOp.Set(3, List.of(new UiProto.Prop.Y(-r.y()))),
                new UiProto.TreeOp.Set(4, List.of(new UiProto.Prop.Y(40 - r.y()))))), new UiProto.Commit(1));
        tick();
        pass();
        assertEquals(0x00FFFF, clientPixel(lx, ly), "confirmed: no jump back");
        assertEquals(0x00FFFF, serverPixel(lx, ly));
    }

    @Test
    void typingShowsInTheFocusedFieldBeforeTheAppAnswers() throws Exception {
        UiConnection shell = open(10, 2, UiProto.Role.SHELL);
        UiConnection app = open(11, 2);
        UiGoldenTest.drain(app);
        send(app, new UiProto.SurfaceCreate(1, UiProto.SurfaceKind.TREE, UiProto.SurfaceRole.WINDOW, UiProto.Layer.NONE, 0, 0, 0, 0, 0,
                UiProto.PixelFormat.NONE, 1, "field"));
        shellPlace(shell);
        send(shell, new UiProto.FocusWindow(1));
        int serial = UiCompositorTest.got(app, UiProto.Configure.class).get(0).serial();
        send(app, new UiProto.TreeOps(1, List.of(
                        new UiProto.TreeOp.Create(1, 0, UiCodec.APPEND, UiProto.NodeKind.RECT, List.of(new UiProto.Prop.W(200),
                                new UiProto.Prop.H(30), new UiProto.Prop.Fill(0xFFFFFFFF))),
                        new UiProto.TreeOp.Create(2, 0, UiCodec.APPEND, UiProto.NodeKind.TEXT, List.of(new UiProto.Prop.X(4),
                                new UiProto.Prop.Y(8), new UiProto.Prop.Text(""), new UiProto.Prop.Color(0x000000FF),
                                new UiProto.Prop.Caret(0))))),
                new UiProto.AckConfigure(1, serial), new UiProto.Commit(1));
        tick();
        pass();
        int sc = comp.display().scale();
        int dark = 0;
        ClientUiScenes.typed(computer, 0, "WWW", host.now);
        ClientUiScenes.tick(host.now);
        for (int x = 4 * sc; x < 40 * sc; x++) for (int y = 8 * sc; y < 22 * sc; y++) if (clientPixel(x, y) != 0xFFFFFF) dark++;
        assertTrue(dark > 10, "the typed text shows in the field at once (" + dark + " ink pixels)");
        for (int x = 4 * sc; x < 40 * sc; x++) assertEquals(0xFFFFFF, serverPixel(x, 14 * sc), "the server still has the empty field");
        // The app's answer replaces the prediction (here it says "W").
        send(app, new UiProto.TreeOps(1, List.of(new UiProto.TreeOp.Set(2, List.of(new UiProto.Prop.Text("W"), new UiProto.Prop.Caret(1))))),
                new UiProto.Commit(1));
        tick();
        pass();
        for (int x = 0; x < 320 * sc; x += 3) {
            assertEquals(serverPixel(x, 14 * sc), clientPixel(x, 14 * sc), "after the app's commit the client shows exactly its text");
        }
    }
}
