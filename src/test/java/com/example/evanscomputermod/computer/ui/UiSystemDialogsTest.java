package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.ui.proto.UiProto;
import org.junit.jupiter.api.Test;

import java.util.List;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

/**
 * P4 glue for the system file picker and "open with" chooser (docs/gui/APPS.md
 * "File picker and chooser"): {@code PickFile} starts {@code picker} with a
 * token; {@code picked TOKEN PATH} from that program (and only that program,
 * once) becomes {@code FilePicked{req, path}} for the app plus a one-file
 * grant; cancels, a picker that exits without answering and one that can't
 * start all answer "". The chooser's {@code chosen TOKEN APP [always]} opens the file.
 */
public class UiSystemDialogsTest {
    final UiCompositorTest t = new UiCompositorTest();
    final UiTestHost host = t.host;

    static String arg(List<String> args, String name) {
        int i = args.indexOf(name);
        return i >= 0 && i + 1 < args.size() ? args.get(i + 1) : null;
    }

    /** An app (pid 20) with a mapped window that sends PickFile; the picker gets pid 50. */
    UiConnection shell;

    UiConnection appPicks(Compositor c, UiProto.PickMode mode) throws Exception {
        shell = t.open(c, 10, UiProto.Role.SHELL);
        UiConnection app = t.mappedApp(c, shell, 20, 1, 0, 0, 200, 100);
        host.nextPid = 50;
        assertEquals(0, app.send(UiCompositorTest.enc(new UiProto.PickFile(7, mode, List.of(".txt", ".md"), "notes.txt"))));
        return app;
    }

    @Test
    void pickedFileReachesTheAppWithAGrant() throws Exception {
        Compositor c = t.comp(UiLimits.defaults());
        UiConnection app = appPicks(c, UiProto.PickMode.SAVE);
        assertEquals("picker", host.spawned.get(host.spawned.size() - 1));
        List<String> args = host.spawnArgs.get(host.spawnArgs.size() - 1);
        assertEquals("save", arg(args, "--mode"));
        assertEquals(".txt,.md", arg(args, "--types"));
        assertEquals("notes.txt", arg(args, "--suggested"));
        String token = arg(args, "--req");
        assertEquals(16, token.length(), "a random token");
        assertEquals(1, c.dialogs().pendingCount());

        // Control: another program can't answer for the picker, and a wrong token is refused.
        assertTrue(c.systemAnswer(20, "picked " + token + " /home/x.txt").startsWith("gui: "));
        assertTrue(c.systemAnswer(50, "picked 0000 /home/x.txt").startsWith("gui: "));
        assertTrue(UiCompositorTest.got(app, UiProto.FilePicked.class).isEmpty());

        assertEquals("ok\n", c.systemAnswer(50, "picked " + token + " /home/my notes.txt"));
        List<UiProto.FilePicked> got = UiCompositorTest.got(app, UiProto.FilePicked.class);
        assertEquals(List.of(new UiProto.FilePicked(7, "/home/my notes.txt")), got);
        assertEquals(List.of("20 /home/my notes.txt true"), host.picked);
        // Once only.
        assertTrue(c.systemAnswer(50, "picked " + token + " /home/y.txt").startsWith("gui: "));
        assertEquals(0, c.dialogs().pendingCount());
    }

    @Test
    void cancelExitAndNoPickerAllAnswerEmpty() throws Exception {
        Compositor c = t.comp(UiLimits.defaults());
        UiConnection app = appPicks(c, UiProto.PickMode.OPEN);
        String token = arg(host.spawnArgs.get(host.spawnArgs.size() - 1), "--req");
        assertEquals("ok\n", c.systemAnswer(50, "picked " + token + " cancel"));
        assertEquals(List.of(new UiProto.FilePicked(7, "")), UiCompositorTest.got(app, UiProto.FilePicked.class));
        assertTrue(host.picked.isEmpty(), "no grant for a cancel");

        // The picker exits without answering.
        host.nextPid = 51;
        assertEquals(0, app.send(UiCompositorTest.enc(new UiProto.PickFile(8, UiProto.PickMode.OPEN, List.of(), ""))));
        c.processExited(51);
        assertEquals(List.of(new UiProto.FilePicked(8, "")), UiCompositorTest.got(app, UiProto.FilePicked.class));

        // No picker program at all.
        host.nextPid = -1;
        assertEquals(0, app.send(UiCompositorTest.enc(new UiProto.PickFile(9, UiProto.PickMode.OPEN, List.of(), ""))));
        assertEquals(List.of(new UiProto.FilePicked(9, "")), UiCompositorTest.got(app, UiProto.FilePicked.class));
        assertTrue(host.picked.isEmpty());
    }

    @Test
    void pickerWindowIsAttachedAndGoesWithItsApp() throws Exception {
        Compositor c = t.comp(UiLimits.defaults());
        appPicks(c, UiProto.PickMode.OPEN);
        UiCompositorTest.got(shell, UiProto.WindowOpened.class);
        int appWindow = c.windows().stream().filter(w -> w.pid() == 20).findFirst().orElseThrow().id();
        // The picker (pid 50) opens its window: the shell sees it as the app window's child.
        UiConnection picker = t.open(c, 50, UiProto.Role.APP);
        assertEquals(0, picker.send(UiCompositorTest.enc(UiCompositorTest.window(1, "Open a file"))));
        List<UiProto.WindowOpened> opened = UiCompositorTest.got(shell, UiProto.WindowOpened.class);
        assertEquals(1, opened.size());
        assertEquals(appWindow, opened.get(0).parent(), "attached to the app's window");
        // The requester exits: its picker is killed.
        c.processExited(20);
        assertTrue(host.killed.contains(50), "picker killed with its app");
        assertEquals(0, c.dialogs().pendingCount());
    }

    @Test
    void chooserOpensWithTheChosenApp() throws Exception {
        Compositor c = t.comp(UiLimits.defaults());
        host.nextPid = 60;
        assertTrue(c.startChooser(0, "/home/a.txt"));
        assertEquals("chooser", host.spawned.get(host.spawned.size() - 1));
        List<String> args = host.spawnArgs.get(host.spawnArgs.size() - 1);
        assertEquals("/home/a.txt", arg(args, "--path"));
        String token = arg(args, "--req");
        assertTrue(c.systemAnswer(60, "picked " + token + " /x").startsWith("gui: "), "a chooser token isn't a picker's");
        assertEquals("ok\n", c.systemAnswer(60, "chosen " + token + " dev.ecm.notes always"));
        assertEquals(List.of("/home/a.txt dev.ecm.notes true"), host.opened);
        // Cancel opens nothing.
        host.nextPid = 61;
        c.startChooser(0, "/home/b.txt");
        String t2 = arg(host.spawnArgs.get(host.spawnArgs.size() - 1), "--req");
        assertEquals("ok\n", c.systemAnswer(61, "chosen " + t2 + " cancel"));
        assertEquals(1, host.opened.size());
    }
}
