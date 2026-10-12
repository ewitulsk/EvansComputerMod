package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.ui.proto.UiCodec;
import com.example.evanscomputermod.computer.ui.proto.UiProto;
import org.junit.jupiter.api.Test;

import java.util.ArrayList;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * The compositor's protocol rules without a game or kernel: strict errors
 * close only the offending connection (ROADMAP P2 "Done when"), every §6
 * limit, configure/ack/commit, frame pacing, chords and keys, popups,
 * pointer lock, damage, safe mode with the restart backoff, and legacy
 * windows.
 */
public class UiCompositorTest {
    final UiTestHost host = new UiTestHost();

    Compositor comp(UiLimits limits) {
        Compositor c = new Compositor(host, UiDisplay.terminal(), limits, 30, "test-shell", List.of(), "ecm-default");
        c.setVisible(true);
        return c;
    }

    static byte[] enc(UiProto.Message... ms) {
        return UiCodec.encodeAll(List.of(ms));
    }

    static UiProto.Message hello(UiProto.Role r) {
        return new UiProto.Hello(UiProto.VERSION, r, List.of());
    }

    static UiProto.SurfaceCreate window(int id, String title) {
        return new UiProto.SurfaceCreate(id, UiProto.SurfaceKind.TREE, UiProto.SurfaceRole.WINDOW, UiProto.Layer.NONE, 0, 0, 0,
                0, 0, UiProto.PixelFormat.NONE, 1, title);
    }

    static UiProto.TreeOp rect(int id, int x, int y, int w, int h, int fill) {
        return new UiProto.TreeOp.Create(id, 0, UiCodec.APPEND, UiProto.NodeKind.RECT,
                List.of(new UiProto.Prop.X(x), new UiProto.Prop.Y(y), new UiProto.Prop.W(w), new UiProto.Prop.H(h),
                        new UiProto.Prop.Fill(fill)));
    }

    UiConnection open(Compositor c, int pid, UiProto.Role role) {
        int[] err = new int[1];
        UiConnection conn = c.open(pid, role, err);
        assertNotNull(conn, "ui_open: " + err[0]);
        assertEquals(0, conn.send(enc(hello(role))));
        return conn;
    }

    static <T extends UiProto.Message> List<T> got(UiConnection c, Class<T> type) throws Exception {
        List<T> out = new ArrayList<>();
        for (UiProto.Message m : UiGoldenTest.drain(c)) if (type.isInstance(m)) out.add(type.cast(m));
        return out;
    }

    /** An app with one configured, committed tree window placed by the shell. */
    UiConnection mappedApp(Compositor c, UiConnection shell, int pid, int window, int x, int y, int w, int h) throws Exception {
        UiConnection app = open(c, pid, UiProto.Role.APP);
        assertEquals(0, app.send(enc(window(1, "app" + pid))));
        assertEquals(0, shell.send(enc(new UiProto.Place(window, x, y, w, h, 0, new UiProto.Transition(0, UiProto.Easing.LINEAR)))));
        List<UiProto.Configure> cfg = got(app, UiProto.Configure.class);
        assertEquals(1, cfg.size());
        assertEquals(w, cfg.get(0).w());
        assertEquals(0, app.send(enc(new UiProto.TreeOps(1, List.of(rect(1, 0, 0, w, h, 0x336699FF))),
                new UiProto.AckConfigure(1, cfg.get(0).serial()), new UiProto.Commit(1))));
        return app;
    }

    @Test
    void malformedMessageClosesOnlyThatConnection() throws Exception {
        Compositor c = comp(UiLimits.defaults());
        UiConnection shell = open(c, 10, UiProto.Role.SHELL);
        UiConnection good = mappedApp(c, shell, 11, 1, 0, 0, 200, 100);
        UiConnection bad = mappedApp(c, shell, 12, 2, 200, 0, 200, 100);
        UiGoldenTest.drain(shell);
        // Garbage: an unknown opcode.
        assertEquals(UiConnection.E_INVALID, bad.send(new byte[]{(byte) 0xEE, (byte) 0xEE, 0, 0, 0, 0, 0, 0}));
        List<UiProto.Message> last = UiGoldenTest.drain(bad);
        assertTrue(last.get(last.size() - 1) instanceof UiProto.Error e && e.code() == UiProto.ErrorCode.UNKNOWN_OPCODE, "Error then close: " + last);
        assertNull(bad.recv(1024, 0), "closed and drained: ui_recv reports -5");
        assertEquals(UiConnection.E_INVALID, bad.send(enc(new UiProto.Commit(1))), "a closed connection takes nothing");
        // Its window is gone (the shell is told); the other app and its window are untouched.
        assertEquals(List.of(2), got(shell, UiProto.WindowClosed.class).stream().map(UiProto.WindowClosed::window).toList());
        assertEquals(1, c.windows().size());
        assertTrue(c.windows().get(0).mapped());
        assertEquals(0, good.send(enc(new UiProto.RequestFrame(1))));
        assertFalse(good.isClosed());
        assertFalse(shell.isClosed());
    }

    @Test
    void everyProtocolErrorHasItsCode() throws Exception {
        Compositor c = comp(UiLimits.defaults().withNodesPerSurface(4).withResourceBytes(64 * 1024));
        UiConnection shell = open(c, 10, UiProto.Role.SHELL);
        record Case(String what, UiProto.ErrorCode code, UiProto.Message... msgs) {}
        UiProto.TreeOp[] five = new UiProto.TreeOp[5];
        for (int i = 0; i < 5; i++) five[i] = rect(i + 1, 0, 0, 1, 1, 0);
        List<Case> cases = List.of(
                new Case("node limit per surface", UiProto.ErrorCode.LIMIT, window(1, "w"), new UiProto.TreeOps(1, List.of(five)), new UiProto.Commit(1)),
                new Case("pixel surface larger than the display", UiProto.ErrorCode.LIMIT,
                        new UiProto.SurfaceCreate(1, UiProto.SurfaceKind.PIXELS, UiProto.SurfaceRole.WINDOW, UiProto.Layer.NONE, 0, 0, 0, 641, 10, UiProto.PixelFormat.RGB565, 1, "big")),
                new Case("resource budget", UiProto.ErrorCode.LIMIT,
                        new UiProto.ResourceCreate(1, UiProto.ResourceKind.IMAGE, 256, 256, UiProto.PixelFormat.RGBA8888)),
                new Case("unknown surface", UiProto.ErrorCode.UNKNOWN_OBJECT, new UiProto.Commit(7)),
                new Case("unknown node", UiProto.ErrorCode.UNKNOWN_OBJECT, window(1, "w"),
                        new UiProto.TreeOps(1, List.of(new UiProto.TreeOp.Remove(9))), new UiProto.Commit(1)),
                new Case("ack for a serial never sent", UiProto.ErrorCode.BAD_STATE, window(1, "w"), new UiProto.AckConfigure(1, 99)),
                new Case("shell message from an app", UiProto.ErrorCode.NOT_PERMITTED, new UiProto.FocusWindow(1)),
                new Case("layer surface from an app", UiProto.ErrorCode.NOT_PERMITTED,
                        new UiProto.SurfaceCreate(1, UiProto.SurfaceKind.TREE, UiProto.SurfaceRole.LAYER, UiProto.Layer.TOP, 0, 0, 0, 0, 0, UiProto.PixelFormat.NONE, 1, "x")),
                new Case("compositor-to-client message", UiProto.ErrorCode.BAD_STATE, new UiProto.Frame(1, 0)),
                new Case("unsupported font size", UiProto.ErrorCode.MALFORMED, window(1, "w"),
                        new UiProto.TreeOps(1, List.of(new UiProto.TreeOp.Create(1, 0, UiCodec.APPEND, UiProto.NodeKind.TEXT,
                                List.of(new UiProto.Prop.Font(UiProto.Font.MONO), new UiProto.Prop.Size(12))))), new UiProto.Commit(1)));
        int pid = 100;
        for (Case k : cases) {
            UiConnection app = open(c, pid++, UiProto.Role.APP);
            app.send(enc(k.msgs()));
            assertTrue(app.isClosed(), k.what() + ": connection closed");
            assertEquals(k.code(), app.closeReason().code, k.what() + ": " + app.closeReason().getMessage());
            assertFalse(shell.isClosed());
        }
        // Hello rules and the version handshake.
        int[] err = new int[1];
        UiConnection noHello = c.open(200, UiProto.Role.APP, err);
        noHello.send(enc(window(1, "w")));
        assertEquals(UiProto.ErrorCode.BAD_STATE, noHello.closeReason().code);
        UiConnection oldVersion = c.open(201, UiProto.Role.APP, err);
        oldVersion.send(enc(new UiProto.Hello(9, UiProto.Role.APP, List.of())));
        assertEquals(UiProto.ErrorCode.VERSION, oldVersion.closeReason().code);
        assertNull(c.open(202, UiProto.Role.SHELL, err), "only one shell");
        assertEquals(UiConnection.E_BUSY, err[0]);
    }

    @Test
    void treeOpsOverTheRateAreMergedNotRejected() throws Exception {
        Compositor c = comp(UiLimits.defaults().withTreeOpsRate(1024));
        UiConnection shell = open(c, 10, UiProto.Role.SHELL);
        UiConnection app = mappedApp(c, shell, 11, 1, 0, 0, 100, 100);
        List<UiProto.TreeOp> many = new ArrayList<>();
        for (int i = 0; i < 200; i++) many.add(rect(10 + i, i, 0, 1, 1, 0xFFFFFFFF));
        assertEquals(0, app.send(enc(new UiProto.TreeOps(1, many), new UiProto.Commit(1))));
        assertFalse(app.isClosed(), "over the rate is not an error");
        host.now += 50;
        c.tick();
        assertEquals((byte) 0x33, c.frameRgba()[50 * 4], "the commit is deferred, not applied");
        assertEquals(0, app.send(enc(new UiProto.TreeOps(1, List.of(rect(500, 0, 0, 1, 1, 0))), new UiProto.Commit(1))));
        host.now += 10_000;
        c.tick();
        assertFalse(app.isClosed());
        assertEquals((byte) 0xFF, c.frameRgba()[50 * 4], "both commits applied once the budget refilled");
    }

    @Test
    void framesOnlyForVisibleSurfaces() throws Exception {
        Compositor c = comp(UiLimits.defaults());
        UiConnection shell = open(c, 10, UiProto.Role.SHELL);
        UiConnection app = mappedApp(c, shell, 11, 1, 0, 0, 100, 100);
        UiGoldenTest.drain(app);
        app.send(enc(new UiProto.RequestFrame(1)));
        host.now += 33;
        c.tick();
        assertEquals(1, got(app, UiProto.Frame.class).size(), "visible: one Frame");
        shell.send(enc(new UiProto.SetVisible(1, false)));
        app.send(enc(new UiProto.RequestFrame(1)));
        host.now += 33;
        c.tick();
        assertEquals(0, got(app, UiProto.Frame.class).size(), "hidden: no Frame");
        shell.send(enc(new UiProto.SetVisible(1, true)));
        host.now += 33;
        c.tick();
        assertEquals(1, got(app, UiProto.Frame.class).size(), "shown again: the pending Frame arrives");
        // Covered by an opaque pixel window: no Frame either.
        UiConnection game = open(c, 12, UiProto.Role.APP);
        game.send(enc(new UiProto.SurfaceCreate(1, UiProto.SurfaceKind.PIXELS, UiProto.SurfaceRole.WINDOW, UiProto.Layer.NONE, 0, 0, 0, 64, 64,
                UiProto.PixelFormat.RGB565, 1, "game")));
        shell.send(enc(new UiProto.Place(2, 0, 0, 640, 400, 0, new UiProto.Transition(0, UiProto.Easing.LINEAR))));
        int serial = got(game, UiProto.Configure.class).get(0).serial();
        game.send(enc(new UiProto.AckConfigure(1, serial), new UiProto.Commit(1)));
        app.send(enc(new UiProto.RequestFrame(1)));
        host.now += 33;
        c.tick();
        assertEquals(0, got(app, UiProto.Frame.class).size(), "covered: no Frame");
    }

    @Test
    void chordsGoToTheShellKeysAndTextToTheFocusedWindow() throws Exception {
        Compositor c = comp(UiLimits.defaults());
        UiConnection shell = open(c, 10, UiProto.Role.SHELL);
        UiConnection app = mappedApp(c, shell, 11, 1, 0, 0, 100, 100);
        shell.send(enc(new UiProto.FocusWindow(1), new UiProto.BindChord(5, UiProto.Mods.ALT, List.of(0x2B))));
        UiGoldenTest.drain(app);
        UiGoldenTest.drain(shell);
        assertTrue(c.key(0x2B, UiProto.Mods.ALT, true, false));
        assertTrue(c.key(0x2B, UiProto.Mods.ALT, false, false));
        assertEquals(List.of(5), got(shell, UiProto.Chord.class).stream().map(UiProto.Chord::id).toList());
        c.key(0x04, 0, true, false);
        c.text("aé");
        List<UiProto.Message> msgs = UiGoldenTest.drain(app);
        assertTrue(msgs.stream().noneMatch(m -> m instanceof UiProto.Key k && k.key() == 0x2B), "the chord's key is swallowed");
        assertTrue(msgs.stream().anyMatch(m -> m instanceof UiProto.Key k && k.key() == 0x04 && k.down()));
        assertTrue(msgs.stream().anyMatch(m -> m instanceof UiProto.Text t && t.text().equals("aé")));
    }

    @Test
    void popupsCloseOnAClickOutsideAndPointerLockFollowsFocus() throws Exception {
        Compositor c = comp(UiLimits.defaults());
        UiConnection shell = open(c, 10, UiProto.Role.SHELL);
        UiConnection app = mappedApp(c, shell, 11, 1, 0, 0, 200, 200);
        shell.send(enc(new UiProto.FocusWindow(1)));
        app.send(enc(new UiProto.SurfaceCreate(2, UiProto.SurfaceKind.TREE, UiProto.SurfaceRole.POPUP, UiProto.Layer.NONE, 1, 10, 10, 50, 50,
                UiProto.PixelFormat.NONE, 1, "menu")));
        int serial = got(app, UiProto.Configure.class).get(0).serial();
        app.send(enc(new UiProto.TreeOps(2, List.of(rect(1, 0, 0, 50, 50, 0xFF0000FF))), new UiProto.AckConfigure(2, serial), new UiProto.Commit(2)));
        c.pointer(UiProto.PointerKind.DOWN, 20, 20, 1, UiProto.Button.LEFT, 0);
        c.pointer(UiProto.PointerKind.UP, 20, 20, 0, UiProto.Button.LEFT, 0);
        List<UiProto.Message> inside = UiGoldenTest.drain(app);
        assertTrue(inside.stream().noneMatch(m -> m instanceof UiProto.PopupDone), "a click inside keeps the popup");
        assertTrue(inside.stream().anyMatch(m -> m instanceof UiProto.Pointer p && p.surface() == 2 && p.x() == 10), "popup-local coordinates");
        c.pointer(UiProto.PointerKind.DOWN, 150, 150, 1, UiProto.Button.LEFT, 0);
        assertEquals(1, got(app, UiProto.PopupDone.class).size());
        // Pointer lock: granted to the focused window, Escape releases it.
        app.send(enc(new UiProto.PointerLock(1, true)));
        assertTrue(got(app, UiProto.PointerLockChanged.class).get(0).locked());
        assertTrue(c.key(Compositor.HID_ESCAPE, 0, true, false));
        assertFalse(got(app, UiProto.PointerLockChanged.class).get(0).locked());
    }

    @Test
    void aSmallChangeDamagesASmallArea() throws Exception {
        Compositor c = comp(UiLimits.defaults());
        UiConnection shell = open(c, 10, UiProto.Role.SHELL);
        UiConnection app = mappedApp(c, shell, 11, 1, 100, 100, 300, 200);
        c.tick();
        host.presents.clear();
        app.send(enc(new UiProto.TreeOps(1, List.of(rect(2, 10, 10, 8, 8, 0xFFFFFFFF))), new UiProto.Commit(1)));
        c.tick();
        assertEquals(1, host.presents.size());
        long area = host.presents.get(0).stream().mapToLong(Box::area).sum();
        assertEquals(64, area, "only the new 8x8 rect: " + host.presents.get(0));
        assertEquals(new Box(110, 110, 118, 118), host.presents.get(0).get(0));
    }

    @Test
    void shellLossStartsSafeModeAndRestartsWithBackoff() throws Exception {
        Compositor c = comp(UiLimits.defaults());
        host.nextPid = 50;
        c.startShell();
        assertEquals(50, c.shellPid());
        UiConnection shell = open(c, 50, UiProto.Role.SHELL);
        UiConnection app = mappedApp(c, shell, 11, 1, 10, 10, 100, 100);
        assertFalse(c.safeMode());
        // The shell process dies.
        c.processExited(50);
        assertTrue(c.safeMode());
        Compositor.WindowInfo w = c.windows().get(0);
        assertEquals(0, w.x());
        assertEquals(Compositor.SAFE_BAR_H, w.y());
        assertTrue((w.states() & UiProto.WindowStates.MAXIMIZED) != 0, "focused window maximized");
        assertEquals(640, got(app, UiProto.Configure.class).get(0).w());
        host.now += 499;
        c.tick();
        assertEquals(1, host.spawned.size(), "not restarted before 0.5 s");
        host.now += 1;
        c.tick();
        assertEquals(2, host.spawned.size(), "restarted after 0.5 s");
        UiConnection shell2 = open(c, 51, UiProto.Role.SHELL);
        assertFalse(c.safeMode(), "back to normal once the shell connects");
        assertEquals(1, got(shell2, UiProto.WindowOpened.class).size(), "the new shell learns about the open window");
        // It dies again quickly: the next delay is 5 s.
        shell2.close();
        assertTrue(c.safeMode());
        host.now += 4999;
        c.tick();
        assertEquals(2, host.spawned.size());
        host.now += 1;
        c.tick();
        assertEquals(3, host.spawned.size());
        // A shell that stops reading is killed and replaced.
        UiConnection shell3 = open(c, 52, UiProto.Role.SHELL);
        app.send(enc(new UiProto.SurfaceSetTitle(1, "renamed")));
        host.now += 5001;
        c.tick();
        assertTrue(c.safeMode(), "unresponsive shell: safe mode");
        assertTrue(host.killed.contains(52), "and it is killed: " + host.killed);
        assertTrue(shell3.isClosed());
    }

    @Test
    void legacyWindowsTakeFramesAndPointerInBufferPixels() throws Exception {
        Compositor c = comp(UiLimits.defaults());
        UiConnection shell = open(c, 10, UiProto.Role.SHELL);
        List<int[]> events = new ArrayList<>();
        UiConnection legacy = c.openLegacy(30, "gba", 240, 160, 2, new UiConnection.LegacySink() {
            @Override public void pointer(int kind, int x, int y, int buttons, int button, int scroll) { events.add(new int[]{kind, x, y}); }
            @Override public void key(int hid, int mods, boolean down) {}
        });
        assertEquals(1, got(shell, UiProto.WindowOpened.class).size());
        byte[] px = new byte[240 * 160 * 2];
        java.util.Arrays.fill(px, (byte) 0xFF);
        c.legacyFrame(legacy, px, 2, null, 240, 160, null);
        shell.send(enc(new UiProto.Place(1, 0, 0, 480, 320, 0, new UiProto.Transition(0, UiProto.Easing.LINEAR))));
        c.tick();
        Compositor.WindowInfo w = c.windows().get(0);
        assertTrue(w.mapped() && w.legacy());
        byte[] f = c.frameRgba();
        assertEquals((byte) 0xFF, f[(300 * 640 + 470) * 4], "scaled x2 to 480x320");
        c.pointer(UiProto.PointerKind.MOVE, 100, 50, 0, UiProto.Button.NONE, 0);
        int[] last = events.get(events.size() - 1);
        assertEquals(50, last[1], "x in the program's pixels (scale 2)");
        assertEquals(25, last[2]);
    }
}
