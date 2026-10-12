package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.ui.font.UiFonts;
import com.example.evanscomputermod.computer.ui.proto.UiCodec;
import com.example.evanscomputermod.computer.ui.proto.UiProto;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * The retained display-list tree of one tree surface (§6): nodes by id under
 * an implicit root group (id 0). {@link #apply} runs one TreeOp with the
 * protocol's checks and reports the damage it caused in surface lu.
 *
 * <p>Geometry: a node's {@code x, y} are in its parent group's content
 * space. A group's {@code (x, y, w, h)} is its box in its own parent's space;
 * its children's coordinates are multiplied by its {@code scale} from the
 * group's origin, and with {@code clip} set they are clipped to the group's
 * box.
 */
public final class NodeTree {

    /** One node. Defaults are the protocol's (§6 "A prop a node never set has its default"). */
    public static final class Node {
        public final int id;
        public final UiProto.NodeKind kind;
        Node parent;
        final List<Node> children;
        public int x, y, w, h;
        public boolean clip;
        public int opacity = 255, scale = 1, hitId;
        public int fill, radius, borderW, borderColor;
        public String text = "";
        public UiProto.Font font = UiProto.Font.UI;
        public int size = 12;
        public int color = 0x000000FF;
        public int resource;
        public UiProto.Rect src;
        public UiProto.Sampling sampling = UiProto.Sampling.NEAREST;
        public int tint = 0xFFFFFFFF;
        public List<UiProto.Point> points = List.of();
        public int width = 1;

        Node(int id, UiProto.NodeKind kind) {
            this.id = id;
            this.kind = kind;
            this.children = kind == UiProto.NodeKind.GROUP ? new ArrayList<>() : null;
        }

        public List<Node> children() {
            return children == null ? List.of() : children;
        }

        public Node parent() {
            return parent;
        }

        /** The text face, or null if the font has no such size. */
        public UiFonts.Face face() {
            return UiFonts.face(font, size);
        }
    }

    private final Node root = new Node(UiCodec.ROOT_NODE, UiProto.NodeKind.GROUP);
    private final Map<Integer, Node> nodes = new HashMap<>();

    public NodeTree() {
        nodes.put(0, root);
    }

    public Node root() {
        return root;
    }

    /** Nodes excluding the root. */
    public int size() {
        return nodes.size() - 1;
    }

    public Node get(int id) {
        return nodes.get(id);
    }

    /**
     * Apply one op. {@code maxNodes} bounds this tree, {@code budget} the
     * connection (both report a limit error when exceeded); returns the change
     * in node count. Damage (surface lu) is added to {@code damage}.
     */
    public int apply(UiProto.TreeOp op, int maxNodes, int budget, Box.Region damage) throws UiError {
        switch (op) {
            case UiProto.TreeOp.Create c -> {
                if (nodes.containsKey(c.id())) throw UiError.badState("node " + c.id() + " already exists");
                if (size() >= maxNodes) throw UiError.limit("more than " + maxNodes + " nodes in one surface");
                if (budget <= 0) throw UiError.limit("too many nodes in this app");
                Node parent = group(c.parent());
                Node n = new Node(c.id(), c.kind());
                setProps(n, c.props());
                insert(parent, n, c.index());
                nodes.put(n.id, n);
                damage.add(absBox(n));
                return 1;
            }
            case UiProto.TreeOp.Set s -> {
                Node n = node(s.id());
                damage.add(absBox(n));
                for (UiProto.Prop p : s.props()) {
                    if (!UiProto.nodeAllows(n.kind, p.tag())) {
                        throw UiError.malformed("prop " + p.name() + " not allowed on a " + n.kind.wireName + " node");
                    }
                }
                setProps(n, s.props());
                damage.add(absBox(n));
                return 0;
            }
            case UiProto.TreeOp.Move m -> {
                Node n = node(m.id());
                Node parent = group(m.parent());
                for (Node a = parent; a != null; a = a.parent) {
                    if (a == n) throw UiError.badState("node " + n.id + " can't move under itself");
                }
                damage.add(absBox(n));
                n.parent.children.remove(n);
                insert(parent, n, m.index());
                damage.add(absBox(n));
                return 0;
            }
            case UiProto.TreeOp.Remove r -> {
                Node n = node(r.id());
                damage.add(absBox(n));
                n.parent.children.remove(n);
                n.parent = null;
                int[] removed = {0};
                forget(n, removed);
                return -removed[0];
            }
        }
    }

    private void forget(Node n, int[] count) {
        nodes.remove(n.id);
        count[0]++;
        for (Node c : n.children()) forget(c, count);
    }

    private Node node(int id) throws UiError {
        Node n = id == 0 ? null : nodes.get(id);
        if (n == null) throw UiError.unknown("no node " + id);
        return n;
    }

    private Node group(int id) throws UiError {
        Node n = nodes.get(id);
        if (n == null) throw UiError.unknown("no parent node " + id);
        if (n.kind != UiProto.NodeKind.GROUP) throw UiError.badState("node " + id + " is not a group");
        return n;
    }

    private static void insert(Node parent, Node n, int index) {
        int i = index < 0 || index > parent.children.size() ? parent.children.size() : index;
        parent.children.add(i, n);
        n.parent = parent;
    }

    private static void setProps(Node n, List<UiProto.Prop> props) throws UiError {
        for (UiProto.Prop p : props) {
            switch (p) {
                case UiProto.Prop.X v -> n.x = v.value();
                case UiProto.Prop.Y v -> n.y = v.value();
                case UiProto.Prop.W v -> n.w = v.value();
                case UiProto.Prop.H v -> n.h = v.value();
                case UiProto.Prop.Clip v -> n.clip = v.value();
                case UiProto.Prop.Opacity v -> n.opacity = v.value();
                case UiProto.Prop.Scale v -> n.scale = v.value();
                case UiProto.Prop.HitId v -> n.hitId = v.value();
                case UiProto.Prop.Fill v -> n.fill = v.value();
                case UiProto.Prop.Radius v -> n.radius = v.value();
                case UiProto.Prop.BorderW v -> n.borderW = v.value();
                case UiProto.Prop.BorderColor v -> n.borderColor = v.value();
                case UiProto.Prop.Text v -> n.text = v.value();
                case UiProto.Prop.Font v -> n.font = v.value();
                case UiProto.Prop.Size v -> n.size = v.value();
                case UiProto.Prop.Color v -> n.color = v.value();
                case UiProto.Prop.Resource v -> n.resource = v.value();
                case UiProto.Prop.Src v -> n.src = v.value();
                case UiProto.Prop.Sampling v -> n.sampling = v.value();
                case UiProto.Prop.Tint v -> n.tint = v.value();
                case UiProto.Prop.Points v -> n.points = v.value();
                case UiProto.Prop.Width v -> n.width = v.value();
            }
        }
        if (n.kind == UiProto.NodeKind.TEXT && n.face() == null) {
            throw UiError.malformed("font " + n.font.wireName + " has no size " + n.size);
        }
    }

    // ------------------------------------------------------------ geometry

    /** What a node (with its subtree) covers, in its parent's content space. */
    public static Box localBox(Node n) {
        return switch (n.kind) {
            case RECT, IMAGE, CANVAS -> Box.xywh(n.x, n.y, n.w, n.h);
            case TEXT -> {
                UiFonts.Face f = n.face();
                yield f == null || n.text.isEmpty() ? Box.EMPTY : Box.xywh(n.x, n.y, f.textWidth(n.text), f.lineHeight());
            }
            case LINE -> {
                if (n.points.isEmpty()) yield Box.EMPTY;
                int x0 = Integer.MAX_VALUE, y0 = Integer.MAX_VALUE, x1 = Integer.MIN_VALUE, y1 = Integer.MIN_VALUE;
                for (UiProto.Point p : n.points) {
                    x0 = Math.min(x0, p.x());
                    y0 = Math.min(y0, p.y());
                    x1 = Math.max(x1, p.x());
                    y1 = Math.max(y1, p.y());
                }
                int g = n.width / 2 + 1;
                yield new Box(x0 - g, y0 - g, x1 + g + 1, y1 + g + 1);
            }
            case GROUP -> {
                Box b = Box.EMPTY;
                for (Node c : n.children()) b = b.union(localBox(c));
                b = b.scale(n.scale).translate(n.x, n.y);
                if (n.clip) b = b.intersect(Box.xywh(n.x, n.y, n.w, n.h));
                yield b;
            }
        };
    }

    /** {@link #localBox} in surface lu, through every ancestor's offset, scale and clip. */
    public static Box absBox(Node n) {
        Box b = localBox(n);
        for (Node g = n.parent; g != null && !b.isEmpty(); g = g.parent) {
            b = b.scale(g.scale).translate(g.x, g.y);
            if (g.clip) b = b.intersect(Box.xywh(g.x, g.y, g.w, g.h));
        }
        return b;
    }

    /** Every text string under {@code n}, in tree order. */
    public static void collectTexts(Node n, List<String> out) {
        if (n.kind == UiProto.NodeKind.TEXT && !n.text.isEmpty()) out.add(n.text);
        for (Node c : n.children()) collectTexts(c, out);
    }

    /** The centre (surface lu) of the first drawn text node showing {@code text}, or null. */
    public static int[] findText(Node n, String text, boolean contains) {
        if (n.kind == UiProto.NodeKind.TEXT && (contains ? n.text.contains(text) : n.text.equals(text))) {
            Box b = absBox(n);
            if (!b.isEmpty()) return new int[]{(b.x0() + b.x1()) / 2, (b.y0() + b.y1()) / 2};
        }
        for (Node c : n.children()) {
            int[] r = findText(c, text, contains);
            if (r != null) return r;
        }
        return null;
    }

    /** Every node referencing resource {@code id} (image or canvas). */
    public List<Node> usersOf(int id) {
        List<Node> out = new ArrayList<>();
        for (Node n : nodes.values()) {
            if ((n.kind == UiProto.NodeKind.IMAGE || n.kind == UiProto.NodeKind.CANVAS) && n.resource == id) out.add(n);
        }
        return out;
    }
}
