package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.ui.proto.UiProto;
import org.junit.jupiter.api.Test;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;

import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * The compositor's services of ROADMAP P3 (UI_PROTOCOL §10, §12) without a
 * game: the clipboard rule (only right after a paste key or a click in the
 * app's own focused window; background apps can't read it), notifications
 * to the shell and actions back (relaunching an app that quit), permission
 * prompts through the shell, the file picker hook, the shell's own windows
 * (its window frames) reported to it, and minimized windows getting no
 * {@code Frame} while their apps keep running (PC lifecycle).
 */
public class UiServicesTest {
    final UiTestHost host = new UiTestHost();
    final UiCompositorTest t = new UiCompositorTest();

    Compositor comp() {
        Compositor c = new Compositor(host, UiDisplay.terminal(), UiLimits.defaults(), 30, "desktop", List.of(), "ecm-default");
        c.setVisible(true);
        return c;
    }

    UiConnection open(Compositor c, int pid, UiProto.Role role) {
        UiConnection conn = t.open(c, pid, role);
        return conn;
    }

    static byte[] enc(UiProto.Message... ms) {
        return UiCompositorTest.enc(ms);
    }

    static final String MIME = "text/plain;charset=utf-8";

    static UiProto.ClipboardData clip(UiConnection c) throws Exception {
        List<UiProto.ClipboardData> got = UiCompositorTest.got(c, UiProto.ClipboardData.class);
        assertEquals(1, got.size(), "one answer");
        return got.get(0);
    }

    @Test
    void clipboardOnlyRightAfterAPasteKeyOrAClickInTheFocusedWindow() throws Exception {
        Compositor c = comp();
        UiConnection shell = open(c, 10, UiProto.Role.SHELL);
        UiConnection editor = t.mappedApp(c, shell, 11, 1, 0, 0, 300, 200);
        UiConnection term = t.mappedApp(c, shell, 12, 2, 300, 0, 300, 200);
        shell.send(enc(new UiProto.FocusWindow(1)));
        // The editor copies.
        editor.send(enc(new UiProto.ClipboardSet(MIME, "echo hi".getBytes(StandardCharsets.UTF_8))));
        assertEquals("echo hi", c.clipboardText());
        // A background app can't read it.
        UiGoldenTest.drain(term);
        term.send(enc(new UiProto.ClipboardRequest(MIME)));
        assertEquals("", clip(term).mime(), "background app refused");
        // The focused app can't either, without a paste key or click.
        UiGoldenTest.drain(editor);
        editor.send(enc(new UiProto.ClipboardRequest(MIME)));
        assertEquals("", clip(editor).mime(), "no paste key yet");
        // The terminal gets focus and Ctrl+Shift+V: it may read it, once.
        shell.send(enc(new UiProto.FocusWindow(2)));
        c.key(Compositor.HID_V, UiProto.Mods.CTRL | UiProto.Mods.SHIFT, true, false);
        UiGoldenTest.drain(term);
        term.send(enc(new UiProto.ClipboardRequest("text/plain")));
        UiProto.ClipboardData d = clip(term);
        assertEquals(MIME, d.mime(), "text/plain matches text/plain;charset=utf-8");
        assertArrayEquals("echo hi".getBytes(StandardCharsets.UTF_8), d.data());
        term.send(enc(new UiProto.ClipboardRequest(MIME)));
        assertEquals("", clip(term).mime(), "one paste per key press");
        // A click in the focused window (a Paste menu item) arms it too, for a while.
        c.pointer(UiProto.PointerKind.DOWN, 400, 100, 1, UiProto.Button.RIGHT, 0);
        c.pointer(UiProto.PointerKind.UP, 400, 100, 0, UiProto.Button.RIGHT, 0);
        UiGoldenTest.drain(term);
        term.send(enc(new UiProto.ClipboardRequest(MIME)));
        assertEquals(MIME, clip(term).mime());
        c.key(Compositor.HID_V, UiProto.Mods.CTRL, true, false);
        host.now += Compositor.PASTE_WINDOW_MS + 1;
        term.send(enc(new UiProto.ClipboardRequest(MIME)));
        assertEquals("", clip(term).mime(), "too long after the key");
        // A different type isn't converted.
        c.key(Compositor.HID_V, UiProto.Mods.CTRL, true, false);
        term.send(enc(new UiProto.ClipboardRequest("image/png")));
        assertEquals("", clip(term).mime());
    }

    @Test
    void notificationsGoToTheShellAndActionsComeBackRelaunchingTheApp() throws Exception {
        Compositor c = comp();
        UiConnection shell = open(c, 10, UiProto.Role.SHELL);
        host.names.put(11, "editor");
        UiConnection app = open(c, 11, UiProto.Role.APP);
        app.send(enc(new UiProto.Notify("saved", "Saved", "notes.txt", "", List.of("Open"))));
        List<UiProto.NotificationPosted> posted = UiCompositorTest.got(shell, UiProto.NotificationPosted.class);
        assertEquals(1, posted.size());
        assertEquals("editor", posted.get(0).appId());
        int nid = posted.get(0).nid();
        shell.send(enc(new UiProto.NotificationInvoke(nid, "Open")));
        List<UiProto.NotificationAction> acts = UiCompositorTest.got(app, UiProto.NotificationAction.class);
        assertEquals(1, acts.size());
        assertEquals("saved", acts.get(0).tag());
        assertEquals("Open", acts.get(0).action());
        // The app quits; tapping the toast starts it again and hands it the action.
        app.close();
        host.nextPid = 20;
        shell.send(enc(new UiProto.NotificationInvoke(nid, "")));
        assertEquals(List.of("editor"), host.spawned);
        UiConnection again = open(c, 20, UiProto.Role.APP);
        List<UiProto.Message> first = UiGoldenTest.drain(again);
        assertTrue(first.get(0) instanceof UiProto.Welcome);
        assertTrue(first.get(1) instanceof UiProto.NotificationAction a && a.tag().equals("saved") && a.action().isEmpty(),
                "the action right after Welcome: " + first);
    }

    @Test
    void permissionPromptsGoThroughTheShell() throws Exception {
        Compositor c = comp();
        List<String> answers = new ArrayList<>();
        // No shell: denied at once.
        c.requestPermission("dev.pat.game", "network", (allow, remember) -> answers.add(allow + "/" + remember));
        assertEquals(List.of("false/false"), answers);
        UiConnection shell = open(c, 10, UiProto.Role.SHELL);
        int req = c.requestPermission("dev.pat.game", "network", (allow, remember) -> answers.add(allow + "/" + remember));
        List<UiProto.PermissionPrompt> prompts = UiCompositorTest.got(shell, UiProto.PermissionPrompt.class);
        assertEquals(1, prompts.size());
        assertEquals(req, prompts.get(0).req());
        assertEquals("dev.pat.game", prompts.get(0).appId());
        assertEquals("network", prompts.get(0).permission());
        shell.send(enc(new UiProto.AnswerPermission(req, true, true)));
        assertEquals(List.of("false/false", "true/true"), answers);
        // An answer for an unknown request is ignored; a shell that goes away denies what's pending.
        shell.send(enc(new UiProto.AnswerPermission(999, true, false)));
        c.requestPermission("dev.pat.game", "radio", (allow, remember) -> answers.add("radio " + allow));
        shell.close();
        assertEquals("radio false", answers.get(answers.size() - 1));
    }

    @Test
    void filePickerHookAnswersPickFile() throws Exception {
        Compositor c = comp();
        UiConnection shell = open(c, 10, UiProto.Role.SHELL);
        UiConnection app = open(c, 11, UiProto.Role.APP);
        UiGoldenTest.drain(app);
        app.send(enc(new UiProto.PickFile(1, UiProto.PickMode.OPEN, List.of("py"), "")));
        assertEquals("", UiCompositorTest.got(app, UiProto.FilePicked.class).get(0).path(), "the system picker can't start in this test host: cancelled");
        c.setFilePicker((appId, request, answer) -> answer.accept("/home/" + appId + "." + request.types().get(0)));
        app.send(enc(new UiProto.PickFile(2, UiProto.PickMode.OPEN, List.of("py"), "")));
        UiProto.FilePicked p = UiCompositorTest.got(app, UiProto.FilePicked.class).get(0);
        assertEquals(2, p.req());
        assertEquals("/home/pid-11.py", p.path());
        assertFalse(shell.isClosed());
    }

    @Test
    void theShellHearsAboutItsOwnWindowsAndMinimizedWindowsGetNoFrames() throws Exception {
        Compositor c = comp();
        host.names.put(10, "desktop");
        UiConnection shell = open(c, 10, UiProto.Role.SHELL);
        UiConnection app = t.mappedApp(c, shell, 11, 1, 20, 40, 200, 100);
        UiGoldenTest.drain(shell);
        // The shell makes a frame window for it: it is told the frame's window id.
        shell.send(enc(UiCompositorTest.window(5, "frame:1")));
        List<UiProto.WindowOpened> opened = UiCompositorTest.got(shell, UiProto.WindowOpened.class);
        assertEquals(1, opened.size());
        assertEquals(2, opened.get(0).window());
        assertEquals("desktop", opened.get(0).appId());
        assertEquals("frame:1", opened.get(0).title());
        assertTrue(c.windows().stream().anyMatch(w -> w.id() == 2 && w.shell()), "flagged as the shell's own");
        shell.send(enc(new UiProto.Stack(List.of(2, 1))));
        // Minimize (SetVisible false): the app keeps running but gets no Frame (PC lifecycle, §11).
        shell.send(enc(new UiProto.SetVisible(1, false), new UiProto.SetVisible(2, false)));
        UiGoldenTest.drain(app);
        app.send(enc(new UiProto.RequestFrame(1)));
        host.now += 100;
        c.tick();
        assertTrue(UiCompositorTest.got(app, UiProto.Frame.class).isEmpty(), "minimized: no Frame");
        assertEquals(0, app.send(enc(new UiProto.SurfaceSetTitle(1, "still running"))), "the app keeps working");
        assertFalse(app.isClosed());
        shell.send(enc(new UiProto.SetVisible(1, true)));
        host.now += 100;
        c.tick();
        assertEquals(1, UiCompositorTest.got(app, UiProto.Frame.class).size(), "restored: the Frame arrives");
        // Destroying the frame tells the shell.
        UiGoldenTest.drain(shell);
        shell.send(enc(new UiProto.SurfaceDestroy(5)));
        assertEquals(List.of(2), UiCompositorTest.got(shell, UiProto.WindowClosed.class).stream().map(UiProto.WindowClosed::window).toList());
    }
}
