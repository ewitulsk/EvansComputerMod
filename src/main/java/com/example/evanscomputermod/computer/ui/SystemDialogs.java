package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.ui.proto.UiProto;

import java.security.SecureRandom;
import java.util.ArrayList;
import java.util.HexFormat;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.function.BiConsumer;
import java.util.function.Consumer;

/**
 * The system file picker and "open with" chooser (docs/gui/APPS.md "File
 * picker and chooser"): programs drawn by the system, not by the app that
 * asked. A request starts one of them with a fresh random token
 *
 * <pre>
 *   picker  --req TOKEN --app APP_ID --mode open|save [--types .a,.b] [--suggested NAME]
 *   chooser --req TOKEN --path PATH
 * </pre>
 *
 * and the program answers with one line on {@code /dev/uictl}
 * ({@link UiManager#control(int, String)} → {@link #answer}):
 *
 * <pre>
 *   picked TOKEN PATH | picked TOKEN cancel
 *   chosen TOKEN APP_ID [always] | chosen TOKEN cancel
 * </pre>
 *
 * An answer counts only from the pid that was started for that token, and
 * only once; a dialog that exits without answering is a cancel. The callers'
 * callbacks get the answer: the compositor turns a pick into
 * {@code FilePicked} plus a one-file grant ({@link CompositorHost#filePicked})
 * and a choice into {@link CompositorHost#openWith}. This class is the
 * implementation a file-picker hook on the compositor plugs into.
 *
 * <p>Not thread-safe on its own: the {@link Compositor} calls it under its lock.
 */
public final class SystemDialogs {
    public static final String PICKER = "picker", CHOOSER = "chooser";

    private static final SecureRandom RANDOM = new SecureRandom();

    /** A dialog waiting for its answer. */
    private static final class Pending {
        final String token, kind;
        final int requesterPid, parentWindow;
        int pid;
        Consumer<String> onPicked;
        BiConsumer<String, Boolean> onChosen;

        Pending(String token, String kind, int requesterPid, int parentWindow) {
            this.token = token;
            this.kind = kind;
            this.requesterPid = requesterPid;
            this.parentWindow = parentWindow;
        }
    }

    private final CompositorHost host;
    private final Map<String, Pending> pending = new LinkedHashMap<>();

    public SystemDialogs(CompositorHost host) {
        this.host = host;
    }

    private static String token() {
        byte[] b = new byte[8];
        RANDOM.nextBytes(b);
        return HexFormat.of().formatHex(b);
    }

    /**
     * Start the picker for {@code p} from program {@code requesterPid} (app
     * {@code appId}, window {@code parentWindow}). {@code answer} gets the
     * path, or "" when cancelled (also at once if the picker can't start).
     */
    public boolean pick(int requesterPid, String appId, int parentWindow, UiProto.PickFile p, Consumer<String> answer) {
        Pending d = new Pending(token(), PICKER, requesterPid, parentWindow);
        d.onPicked = answer;
        List<String> args = new ArrayList<>(List.of("--req", d.token, "--app", appId == null ? "" : appId,
                "--mode", p.mode() == UiProto.PickMode.SAVE ? "save" : "open"));
        if (!p.types().isEmpty()) args.addAll(List.of("--types", String.join(",", p.types())));
        if (!p.suggested().isEmpty()) args.addAll(List.of("--suggested", p.suggested()));
        return start(d, args, () -> answer.accept(""));
    }

    /**
     * Start the chooser for {@code path} (several apps open it, none is the
     * default). {@code answer} gets (app id, always) or (null, false) when cancelled.
     */
    public boolean choose(int requesterPid, int parentWindow, String path, BiConsumer<String, Boolean> answer) {
        Pending d = new Pending(token(), CHOOSER, requesterPid, parentWindow);
        d.onChosen = answer;
        return start(d, List.of("--req", d.token, "--path", path), () -> answer.accept(null, false));
    }

    private boolean start(Pending d, List<String> args, Runnable cancel) {
        pending.put(d.token, d);
        int pid = host.spawn(d.kind, args);
        if (pid <= 0) {
            pending.remove(d.token);
            host.log("can't start the " + d.kind);
            cancel.run();
            return false;
        }
        d.pid = pid;
        host.log("started the " + d.kind + " as pid " + pid + " for pid " + d.requesterPid);
        return true;
    }

    /**
     * A {@code picked} / {@code chosen} line from program {@code pid}. Returns
     * the reply for {@code /dev/uictl}: {@code "ok\n"} or {@code "gui: ...\n"}.
     */
    public String answer(int pid, String line) {
        String[] w = line.trim().split("\\s+", 3);
        if (w.length < 3) return "gui: usage: picked TOKEN PATH|cancel, chosen TOKEN APP_ID [always]|cancel\n";
        Pending d = pending.get(w[1]);
        String kind = w[0].equals("picked") ? PICKER : w[0].equals("chosen") ? CHOOSER : null;
        if (d == null || !d.kind.equals(kind)) return "gui: no such " + (kind == null ? "dialog" : kind) + " request\n";
        if (d.pid != pid) return "gui: that request belongs to another program\n";
        pending.remove(d.token);
        String rest = w[2].trim();
        if (d.onPicked != null) {
            String path = rest.equals("cancel") ? "" : rest;
            if (!path.isEmpty() && !path.startsWith("/")) path = "";
            d.onPicked.accept(path);
        } else {
            if (rest.equals("cancel")) {
                d.onChosen.accept(null, false);
            } else {
                String[] a = rest.split("\\s+");
                d.onChosen.accept(a[0], a.length > 1 && a[1].equals("always"));
            }
        }
        return "ok\n";
    }

    /** A program exited: its own unanswered dialogs are cancels; dialogs it asked for go away. */
    public void processExited(int pid) {
        for (Pending d : new ArrayList<>(pending.values())) {
            if (d.pid == pid) {
                pending.remove(d.token);
                if (d.onPicked != null) d.onPicked.accept("");
                else d.onChosen.accept(null, false);
            } else if (d.requesterPid == pid) {
                pending.remove(d.token);
                host.kill(d.pid);
            }
        }
    }

    /** The window a dialog program's windows belong to (WindowOpened.parent), or 0. */
    public int parentWindowOf(int pid) {
        for (Pending d : pending.values()) if (d.pid == pid) return d.parentWindow;
        return 0;
    }

    /** Dialogs waiting for an answer (diagnostics, tests). */
    public int pendingCount() {
        return pending.size();
    }
}
