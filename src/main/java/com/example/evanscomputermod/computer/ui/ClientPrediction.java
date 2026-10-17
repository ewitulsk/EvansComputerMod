package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.ui.font.UiFonts;
import com.example.evanscomputermod.computer.ui.proto.UiProto;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Feedback a client shows before the server answers (DISPLAY_PIPELINE_SPEC.md
 * Part F), on its {@link UiSceneMirror}: the props an app put on its nodes
 * (UI protocol v2) say what is safe to predict.
 *
 * <ul>
 *   <li><b>Hover and pressed styles</b>: a node with {@code HoverFill},
 *       {@code HoverColor}, ... uses them while the hit id under the pointer
 *       is its own {@code HitId} (pressed: while the left button went down
 *       on it and is still held).</li>
 *   <li><b>Scroll containers</b>: the wheel over a group with {@code Scroll}
 *       scrolls it here at once (the content shifts by the new offset minus
 *       the {@code ScrollX/ScrollY} the app laid it out at, clamped to
 *       {@code ContentW/ContentH}); the client reports {@code Scrolled} and
 *       the app's next commit confirms it.</li>
 *   <li><b>Window drags</b>: a left drag that starts on a node with
 *       {@code DragWindow} moves that node's window and window
 *       {@code DragWindow} with the pointer; after the release the shell's
 *       placement takes over (or after a second).</li>
 *   <li><b>The caret</b>: typed characters go into the focused window's text
 *       node with {@code Caret} at once, and its {@code CaretOf} rect moves
 *       with them; the app's next change of that text replaces it.</li>
 * </ul>
 *
 * Everything is a temporary change of the mirror's nodes: {@link #undo}
 * puts the server's values back before a message is applied and
 * {@link #redo} predicts again on the result, so what the server says always
 * wins. Client main thread.
 */
public final class ClientPrediction {
    /** Wheel step in lu per notch (ecm-ui scrolls this much). */
    static int WHEEL_STEP_LU = 40;
    /** A prediction the server hasn't confirmed is dropped after this long (ms). */
    static final long EXPIRE_MS = 1000;

    /** A scroll the client did locally: send it to the app as {@code Scrolled}. */
    public record ScrollReport(int surface, int node, int x, int y) {}

    private final UiSceneMirror m;
    // Pointer state, in device pixels.
    private int px = -1, py = -1;
    private boolean leftDown;
    private Surface hoverSurface, pressSurface;
    private int hoverHit, pressHit;
    // Scrolls: by surface and node, the offset shown here and when it was made.
    private record Scroll(Surface s, int node, int x, int y, long atMs) {}
    private final Map<Long, Scroll> scrolls = new HashMap<>();
    // A drag: the windows it moves, where it started, and the offset now.
    private UiWindow dragA, dragB;
    private int dragX0, dragY0, dragDx, dragDy;
    private boolean dragging;
    private long dragEndMs;
    private int dragPlacedX = Integer.MIN_VALUE, dragPlacedY;
    // Typing: the text node, its server text and caret, what was typed after it.
    private Surface typeSurface;
    private int typeNode;
    private String typeServerText;
    private int typeServerCaret;
    private final StringBuilder typed = new StringBuilder();
    private int typedDeletes;
    private long typeAtMs;

    // Undo log: node fields changed by the last redo.
    private record Saved(NodeTree.Node n, int fill, int borderColor, int color, int x, int y, String text) {}
    private final List<Saved> saved = new ArrayList<>();
    private final List<Surface> savedIn = new ArrayList<>();
    private long predictions;

    ClientPrediction(UiSceneMirror m) {
        this.m = m;
    }

    long predictions() {
        return predictions;
    }

    // ------------------------------------------------------------ input

    /**
     * A pointer event from the player (device pixels of the display; kinds
     * as {@code MouseInputPacket}: 1 move, 2 down, 3 up, 4 scroll). Returns a
     * scroll to report when the wheel was used here (the event then doesn't
     * go to the server as a wheel event), else null.
     */
    ScrollReport pointer(int kind, int x, int y, int button, int scrollDir, long clientNowMs) {
        long now = m.sceneTime(clientNowMs);
        undo();
        px = x;
        py = y;
        ScrollReport report = null;
        Hit h = hitAt(x, y, now);
        switch (kind) {
            case 2 -> {
                if (button == 0) {
                    leftDown = true;
                    pressSurface = h == null ? null : h.surface;
                    pressHit = h == null ? 0 : h.hitId;
                    if (h != null && h.drag != null) startDrag(h, x, y);
                }
            }
            case 3 -> {
                if (button == 0) {
                    leftDown = false;
                    pressHit = 0;
                    pressSurface = null;
                    if (dragging) {
                        dragging = false;
                        dragEndMs = clientNowMs;
                    }
                }
            }
            case 4 -> report = scroll(h, scrollDir, clientNowMs);
            default -> {
            }
        }
        if (dragging) {
            dragDx = x - dragX0;
            dragDy = y - dragY0;
        }
        hoverSurface = h == null ? null : h.surface;
        hoverHit = h == null ? 0 : h.hitId;
        redo(clientNowMs);
        return report;
    }

    /** Typed text: predicted into the focused text field. Returns whether it was shown. */
    boolean text(String s, long clientNowMs) {
        if (s == null || s.isEmpty() || s.chars().anyMatch(c -> c < 0x20)) return false;
        undo();
        boolean ok = beginTyping(clientNowMs);
        if (ok) {
            typed.append(s);
            typeAtMs = clientNowMs;
        }
        redo(clientNowMs);
        return ok;
    }

    /** A key (HID usage): Backspace removes a predicted or server character before the caret. */
    boolean key(int hid, boolean down, long clientNowMs) {
        if (!down || hid != 0x2A) return false;
        undo();
        boolean ok = beginTyping(clientNowMs);
        if (ok) {
            if (typed.length() > 0) typed.setLength(typed.length() - 1);
            else typedDeletes++;
            typeAtMs = clientNowMs;
        }
        redo(clientNowMs);
        return ok;
    }

    private boolean beginTyping(long clientNowMs) {
        UiWindow f = m.focusedWindow();
        if (f == null || f.surface.tree == null) return false;
        NodeTree.Node t = findCaret(f.surface.tree.root());
        if (t == null) return false;
        if (typeSurface != f.surface || typeNode != t.id) {
            typeSurface = f.surface;
            typeNode = t.id;
            typeServerText = t.text;
            typeServerCaret = t.caret;
            typed.setLength(0);
            typedDeletes = 0;
        }
        return true;
    }

    private static NodeTree.Node findCaret(NodeTree.Node n) {
        if (n.kind == UiProto.NodeKind.TEXT && n.caret != null) return n;
        for (NodeTree.Node c : n.children()) {
            NodeTree.Node r = findCaret(c);
            if (r != null) return r;
        }
        return null;
    }

    private ScrollReport scroll(Hit h, int dir, long clientNowMs) {
        if (h == null || h.scroll == null || dir == 0) return null;
        NodeTree.Node g = h.scroll;
        long key = key(h.surface, g.id);
        Scroll cur = scrolls.get(key);
        int x = cur != null ? cur.x : g.scrollX, y = cur != null ? cur.y : g.scrollY;
        // Wheel up (dir > 0) shows what is above: the offset goes down.
        int step = -Integer.signum(dir) * WHEEL_STEP_LU;
        if ((g.scroll & 2) != 0) y = clamp(y + step, Math.max(0, g.contentH - g.h));
        else x = clamp(x + step, Math.max(0, g.contentW - g.w));
        scrolls.put(key, new Scroll(h.surface, g.id, x, y, clientNowMs));
        predictions++;
        return new ScrollReport(m.sceneIdOf(h.surface), g.id, x, y);
    }

    private static int clamp(int v, int max) {
        return Math.max(0, Math.min(max, v));
    }

    private static long key(Surface s, int node) {
        return ((long) System.identityHashCode(s) << 32) | (node & 0xFFFFFFFFL);
    }

    private void startDrag(Hit h, int x, int y) {
        if (h.surface.window == null) return;
        dragA = h.surface.window;
        dragB = m.window(h.drag.dragWindow);
        dragX0 = x;
        dragY0 = y;
        dragDx = dragDy = 0;
        dragging = true;
        dragPlacedX = Integer.MIN_VALUE;
        predictions++;
    }

    // ------------------------------------------------------------ hit testing

    /** What is under a device pixel: the surface, its hit id, the scroll group and drag node there. */
    private record Hit(Surface surface, int hitId, NodeTree.Node scroll, NodeTree.Node drag) {}

    private Hit hitAt(int x, int y, long now) {
        List<SceneRenderer.Item> order = m.drawItems(now);
        for (int i = order.size() - 1; i >= 0; i--) {
            SceneRenderer.Item it = order.get(i);
            if (it.surface == null || !it.geo.frame.contains(x, y)) continue;
            Surface s = it.surface;
            if (s.tree == null) return new Hit(s, 0, null, null);
            int lx = Math.floorDiv(x - it.geo.ox, it.geo.k), ly = Math.floorDiv(y - it.geo.oy, it.geo.k);
            int hit = Rasterizer.hitTest(s.tree.root(), lx, ly);
            NodeTree.Node[] found = new NodeTree.Node[2];
            walk(s.tree.root(), 0, 0, 1, null, lx, ly, found);
            return new Hit(s, hit, found[0], found[1]);
        }
        return null;
    }

    /** The deepest scroll group and the topmost drag node containing (px, py), with the drawing geometry. */
    private static void walk(NodeTree.Node g, int tx, int ty, int s, Box clip, int px, int py, NodeTree.Node[] found) {
        int gx = tx + g.x * s, gy = ty + g.y * s, cs = s * g.scale;
        if (g.id == 0) {
            gx = tx;
            gy = ty;
            cs = s;
        }
        Box c = clip;
        if (g.clip && g.id != 0) {
            Box gb = Box.xywh(gx, gy, g.w * s, g.h * s);
            c = c == null ? gb : c.intersect(gb);
        }
        if (c != null && !c.contains(px, py)) return;
        if (g.id != 0) {
            Box gb = Box.xywh(gx, gy, g.w * s, g.h * s);
            if (gb.contains(px, py)) {
                if (g.scroll != 0) found[0] = g;
                if (g.dragWindow != 0 && found[1] == null) found[1] = g;
            }
        }
        List<NodeTree.Node> ch = g.children();
        for (int i = ch.size() - 1; i >= 0; i--) {
            NodeTree.Node n = ch.get(i);
            if (n.kind == UiProto.NodeKind.GROUP) {
                walk(n, gx, gy, cs, c, px, py, found);
            } else if (n.dragWindow != 0 && found[1] == null) {
                Box b = NodeTree.localBox(n).scale(cs).translate(gx, gy);
                if (b.contains(px, py)) found[1] = n;
            }
        }
    }

    // ------------------------------------------------------------ apply / undo

    /** Put the server's values back (before a message is applied, and before predicting again). */
    void undo() {
        // Damage where the predicted values were drawn, then where the server's are.
        for (int i = 0; i < saved.size(); i++) m.damageNode(savedIn.get(i), saved.get(i).n);
        for (int i = saved.size() - 1; i >= 0; i--) {
            Saved v = saved.get(i);
            v.n.fill = v.fill;
            v.n.borderColor = v.borderColor;
            v.n.color = v.color;
            v.n.x = v.x;
            v.n.y = v.y;
            v.n.text = v.text;
        }
        for (int i = 0; i < saved.size(); i++) m.damageNode(savedIn.get(i), saved.get(i).n);
        saved.clear();
        savedIn.clear();
        if (dragA != null) {
            dragA.dragDx = dragA.dragDy = 0;
            if (dragB != null) dragB.dragDx = dragB.dragDy = 0;
            m.damageAll();
        }
    }

    private void save(Surface s, NodeTree.Node n) {
        for (Saved v : saved) if (v.n == n) return;
        saved.add(new Saved(n, n.fill, n.borderColor, n.color, n.x, n.y, n.text));
        savedIn.add(s);
    }

    /** Predict on the mirror's current tree: hover, pressed, scrolls, the drag and typing. */
    void redo(long clientNowMs) {
        if (hoverSurface != null && hoverSurface.tree != null && hoverHit != 0) {
            style(hoverSurface, hoverSurface.tree.root(), hoverHit, false);
        }
        if (leftDown && pressSurface != null && pressSurface.tree != null && pressHit != 0) {
            style(pressSurface, pressSurface.tree.root(), pressHit, true);
        }
        redoScrolls(clientNowMs);
        redoDrag(clientNowMs);
        redoTyping(clientNowMs);
    }

    private void style(Surface s, NodeTree.Node n, int hit, boolean pressed) {
        if (n.hitId == hit) {
            Integer fill = pressed ? n.pressedFill : n.hoverFill;
            Integer border = pressed ? n.pressedBorderColor : n.hoverBorderColor;
            Integer color = pressed ? n.pressedColor : n.hoverColor;
            if (fill != null || border != null || color != null) {
                save(s, n);
                if (fill != null) n.fill = fill;
                if (border != null) n.borderColor = border;
                if (color != null) n.color = color;
                m.damageNode(s, n);
            }
        }
        for (NodeTree.Node c : n.children()) style(s, c, hit, pressed);
    }

    private void redoScrolls(long clientNowMs) {
        for (var it = scrolls.entrySet().iterator(); it.hasNext(); ) {
            Scroll sc = it.next().getValue();
            NodeTree.Node g = sc.s.tree == null ? null : sc.s.tree.get(sc.node);
            if (g == null || (g.scrollX == sc.x && g.scrollY == sc.y) || clientNowMs - sc.atMs > EXPIRE_MS) {
                it.remove(); // the app confirmed it (or it never will)
                continue;
            }
            int dx = sc.x - g.scrollX, dy = sc.y - g.scrollY;
            for (NodeTree.Node c : g.children()) {
                save(sc.s, c);
                c.x -= dx;
                c.y -= dy;
            }
            m.damageNode(sc.s, g);
        }
    }

    private void redoDrag(long clientNowMs) {
        if (dragA == null) return;
        if (!dragging) {
            // After the release: the shell places the windows; stop as soon as it did (or after a second).
            int[] r = dragA.rect(m.sceneTime(clientNowMs));
            if (dragPlacedX == Integer.MIN_VALUE) {
                dragPlacedX = r[0];
                dragPlacedY = r[1];
            }
            boolean moved = r[0] != dragPlacedX || r[1] != dragPlacedY;
            if (moved || clientNowMs - dragEndMs > EXPIRE_MS) {
                dragA = dragB = null;
                m.damageAll();
                return;
            }
        }
        int sc = Math.max(1, m.displayScale());
        dragA.dragDx = dragDx / sc;
        dragA.dragDy = dragDy / sc;
        if (dragB != null) {
            dragB.dragDx = dragA.dragDx;
            dragB.dragDy = dragA.dragDy;
        }
        m.damageAll();
    }

    private void redoTyping(long clientNowMs) {
        if (typeSurface == null || typeSurface.tree == null) return;
        NodeTree.Node t = typeSurface.tree.get(typeNode);
        boolean changed = t == null || t.caret == null || !t.text.equals(typeServerText) || t.caret != typeServerCaret;
        if (changed || clientNowMs - typeAtMs > EXPIRE_MS || (typed.length() == 0 && typedDeletes == 0)) {
            // The app's own change of the field replaces the prediction.
            typeSurface = null;
            typed.setLength(0);
            typedDeletes = 0;
            return;
        }
        String text = t.text;
        int caret = Math.max(0, Math.min(text.codePointCount(0, text.length()), t.caret));
        int at = text.offsetByCodePoints(0, caret);
        int del = Math.min(typedDeletes, caret);
        int from = text.offsetByCodePoints(0, caret - del);
        String removed = text.substring(from, at);
        save(typeSurface, t);
        t.text = text.substring(0, from) + typed + text.substring(at);
        m.damageNode(typeSurface, t);
        UiFonts.Face face = t.face();
        int shift = face == null ? 0 : face.textWidth(typed.toString()) - face.textWidth(removed);
        NodeTree.Node caretRect = findCaretOf(typeSurface.tree.root(), t.id);
        if (caretRect != null && shift != 0) {
            save(typeSurface, caretRect);
            m.damageNode(typeSurface, caretRect);
            caretRect.x += shift;
            m.damageNode(typeSurface, caretRect);
        }
        predictions++;
    }

    private static NodeTree.Node findCaretOf(NodeTree.Node n, int textId) {
        if (n.caretOf != null && n.caretOf == textId) return n;
        for (NodeTree.Node c : n.children()) {
            NodeTree.Node r = findCaretOf(c, textId);
            if (r != null) return r;
        }
        return null;
    }

    /** Is anything predicted now (tests)? */
    boolean active() {
        return !saved.isEmpty() || dragA != null || !scrolls.isEmpty();
    }
}
