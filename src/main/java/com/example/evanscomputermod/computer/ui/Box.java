package com.example.evanscomputermod.computer.ui;

import java.util.ArrayList;
import java.util.List;

/** An integer rectangle {@code [x0, x1) x [y0, y1)}; empty when either side is {@code <= 0}. */
public record Box(int x0, int y0, int x1, int y1) {
    public static final Box EMPTY = new Box(0, 0, 0, 0);

    public static Box xywh(int x, int y, int w, int h) {
        return new Box(x, y, x + Math.max(0, w), y + Math.max(0, h));
    }

    public int w() {
        return x1 - x0;
    }

    public int h() {
        return y1 - y0;
    }

    public boolean isEmpty() {
        return x1 <= x0 || y1 <= y0;
    }

    public long area() {
        return isEmpty() ? 0 : (long) w() * h();
    }

    public Box intersect(Box o) {
        Box b = new Box(Math.max(x0, o.x0), Math.max(y0, o.y0), Math.min(x1, o.x1), Math.min(y1, o.y1));
        return b.isEmpty() ? EMPTY : b;
    }

    public Box union(Box o) {
        if (isEmpty()) return o;
        if (o.isEmpty()) return this;
        return new Box(Math.min(x0, o.x0), Math.min(y0, o.y0), Math.max(x1, o.x1), Math.max(y1, o.y1));
    }

    public boolean intersects(Box o) {
        return x0 < o.x1 && o.x0 < x1 && y0 < o.y1 && o.y0 < y1;
    }

    public boolean contains(int x, int y) {
        return x >= x0 && x < x1 && y >= y0 && y < y1;
    }

    public boolean contains(Box o) {
        return o.isEmpty() || (o.x0 >= x0 && o.y0 >= y0 && o.x1 <= x1 && o.y1 <= y1);
    }

    public Box translate(int dx, int dy) {
        return isEmpty() ? EMPTY : new Box(x0 + dx, y0 + dy, x1 + dx, y1 + dy);
    }

    public Box scale(int s) {
        return isEmpty() ? EMPTY : new Box(x0 * s, y0 * s, x1 * s, y1 * s);
    }

    public Box grow(int n) {
        return isEmpty() ? EMPTY : new Box(x0 - n, y0 - n, x1 + n, y1 + n);
    }

    /**
     * A small set of damage rectangles. Overlapping or touching additions are
     * merged; past {@link #MAX} rectangles everything collapses into one
     * bounding box, which keeps the per-frame work bounded.
     */
    public static final class Region {
        public static final int MAX = 16;
        private final List<Box> boxes = new ArrayList<>();

        public void add(Box b) {
            if (b == null || b.isEmpty()) return;
            boolean merged = true;
            while (merged) {
                merged = false;
                for (int i = 0; i < boxes.size(); i++) {
                    Box o = boxes.get(i);
                    if (o.contains(b)) return;
                    if (touches(o, b)) {
                        boxes.remove(i);
                        b = o.union(b);
                        merged = true;
                        break;
                    }
                }
            }
            boxes.add(b);
            if (boxes.size() > MAX) {
                Box all = EMPTY;
                for (Box o : boxes) all = all.union(o);
                boxes.clear();
                boxes.add(all);
            }
        }

        private static boolean touches(Box a, Box b) {
            return a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1;
        }

        public void addAll(Region r) {
            for (Box b : r.boxes) add(b);
        }

        public boolean isEmpty() {
            return boxes.isEmpty();
        }

        public List<Box> boxes() {
            return boxes;
        }

        public Box bounds() {
            Box all = EMPTY;
            for (Box o : boxes) all = all.union(o);
            return all;
        }

        public void clear() {
            boxes.clear();
        }

        /** Remove and return the rectangles, each clipped to {@code limit}. */
        public List<Box> take(Box limit) {
            List<Box> out = new ArrayList<>();
            for (Box b : boxes) {
                Box c = b.intersect(limit);
                if (!c.isEmpty()) out.add(c);
            }
            boxes.clear();
            return out;
        }
    }
}
