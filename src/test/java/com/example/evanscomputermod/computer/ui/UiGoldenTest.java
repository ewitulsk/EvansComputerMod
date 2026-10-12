package com.example.evanscomputermod.computer.ui;

import com.example.evanscomputermod.computer.ui.proto.UiCodec;
import com.example.evanscomputermod.computer.ui.proto.UiProto;
import org.junit.jupiter.api.DynamicTest;
import org.junit.jupiter.api.TestFactory;

import javax.imageio.ImageIO;
import java.awt.image.BufferedImage;
import java.io.ByteArrayInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNotNull;
import static org.junit.jupiter.api.Assertions.assertTrue;
import static org.junit.jupiter.api.Assertions.fail;

/**
 * Golden images (ROADMAP P2): recorded protocol sessions
 * ({@code src/test/resources/ui-sessions/*.session}, written by
 * {@code cargo run -p ecm-ui --example record_ui_sessions}) are replayed
 * through the real {@link Compositor} and {@link Rasterizer}; every
 * snapshot must equal its checked-in PNG ({@code ui-golden/<name>.png})
 * pixel for pixel, and every recorded pointer event must hit the node the
 * recording expects. No game, no kernel.
 *
 * <p>To (re)create the PNGs after a deliberate rendering change, run with
 * {@code -Decm.ui.updateGoldens=true} (they are written into
 * src/test/resources/ui-golden/) and look at every one before committing.
 */
public class UiGoldenTest {
    static final String[] SESSIONS = {"tiled_shell", "primitives", "primitives_x2", "layers",
            // P4: the App Store at compact / medium / wide (record_app_sessions in ecm-apps-ui), picker, chooser.
            "store_compact", "store_medium", "store_wide", "picker_save", "chooser"};
    static final boolean UPDATE = Boolean.getBoolean("ecm.ui.updateGoldens");

    @TestFactory
    List<DynamicTest> sessions() {
        List<DynamicTest> out = new ArrayList<>();
        for (String s : SESSIONS) out.add(DynamicTest.dynamicTest(s, () -> replay(s)));
        return out;
    }

    static Path repoRoot() {
        Path p = Path.of("").toAbsolutePath();
        while (p != null && !Files.exists(p.resolve("rust/Cargo.toml"))) p = p.getParent();
        return p;
    }

    static byte[] resource(String path) throws IOException {
        try (InputStream in = UiGoldenTest.class.getResourceAsStream(path)) {
            if (in != null) return in.readAllBytes();
        }
        Path root = repoRoot();
        Path f = root == null ? null : root.resolve("src/test/resources").resolve(path.substring(1));
        return f != null && Files.exists(f) ? Files.readAllBytes(f) : null;
    }

    /** Replay one session; returns the compositor for further checks. */
    static Compositor replay(String name) throws Exception {
        return replay(name, null);
    }

    /** Watches a replay (client rendering tests, UiClientRenderTest). */
    interface Hook {
        /** The display to replay on (default: the recorded one). */
        default UiDisplay display(int w, int h, int scale) {
            return new UiDisplay(w, h, scale, scale == 2 ? 3 : 2, UiProto.Device.TERMINAL);
        }

        /** Compare snapshots with the golden PNGs (off for a display other than the recorded one). */
        default boolean goldens() {
            return true;
        }

        /** Recorded pointer positions are multiplied by this (a display at a higher scale). */
        default int pointerScale() {
            return 1;
        }

        default void started(Compositor comp, UiTestHost host) throws Exception {}

        /** After every compositor tick (and before a snapshot's comparison). */
        default void ticked(Compositor comp, UiTestHost host) throws Exception {}

        default void snapshot(String name, Compositor comp, UiTestHost host) throws Exception {}
    }

    /** Replay one session, telling {@code hook} (may be null) what happens. */
    static Compositor replay(String name, Hook hook) throws Exception {
        byte[] data = resource("/ui-sessions/" + name + ".session");
        assertNotNull(data, "session " + name + " missing (record it with cargo run -p ecm-ui --example record_ui_sessions)");
        ByteBuffer b = ByteBuffer.wrap(data).order(ByteOrder.LITTLE_ENDIAN);
        byte[] magic = new byte[8];
        b.get(magic);
        assertEquals("ECMUISS1", new String(magic, StandardCharsets.US_ASCII));
        int w = b.getInt(), h = b.getInt(), scale = b.getInt();
        UiTestHost host = new UiTestHost();
        Hook hk = hook != null ? hook : new Hook() {};
        UiDisplay display = hk.display(w, h, scale);
        Compositor comp = new Compositor(host, display, UiLimits.defaults(), 30, "test-shell", List.of(), "ecm-default");
        comp.setVisible(true);
        hk.started(comp, host);
        Map<Integer, UiConnection> conns = new HashMap<>();
        List<String> failures = new ArrayList<>();
        int record = 0;
        while (b.hasRemaining()) {
            int type = b.get() & 0xFF;
            record++;
            String where = name + " record " + record;
            switch (type) {
                case 1 -> {
                    int id = b.getInt(), pid = b.getInt(), role = b.get();
                    byte[] n = new byte[b.getShort() & 0xFFFF];
                    b.get(n);
                    host.names.put(pid, new String(n, StandardCharsets.UTF_8));
                    int[] err = new int[1];
                    UiConnection c = comp.open(pid, role == 1 ? UiProto.Role.SHELL : UiProto.Role.APP, err);
                    assertNotNull(c, where + ": ui_open failed with " + err[0]);
                    conns.put(id, c);
                }
                case 2 -> {
                    UiConnection c = conns.get(b.getInt());
                    byte[] m = bytes(b);
                    int rc = c.send(m);
                    assertEquals(0, rc, where + ": ui_send rejected: " + reason(c));
                }
                case 3 -> {
                    UiConnection c = conns.get(b.getInt());
                    int target = b.getInt(), n = b.getInt();
                    int[] rects = new int[n * 4];
                    for (int i = 0; i < rects.length; i++) rects[i] = b.getInt();
                    byte[] px = bytes(b);
                    int rc = c.putPixels(target, px.length, rects, (off, dst, dstOff, len) -> System.arraycopy(px, off, dst, dstOff, len));
                    assertEquals(0, rc, where + ": ui_put_pixels rejected: " + reason(c));
                }
                case 4 -> {
                    UiConnection c = conns.get(b.getInt());
                    int res = b.getInt(), kind = b.getInt();
                    byte[] img = bytes(b);
                    assertEquals(0, c.upload(res, kind, img), where + ": ui_upload rejected: " + reason(c));
                }
                case 5 -> {
                    host.now += b.getInt();
                    comp.tick();
                    hk.ticked(comp, host);
                }
                case 6 -> {
                    byte[] n = new byte[b.getShort() & 0xFFFF];
                    b.get(n);
                    comp.tick();
                    hk.ticked(comp, host);
                    String snap = new String(n, StandardCharsets.UTF_8);
                    if (hk.goldens()) {
                        String why = compare(snap, comp.frameRgba(), w, h);
                        if (why != null) failures.add(why);
                    }
                    hk.snapshot(snap, comp, host);
                }
                case 7 -> {
                    UiProto.PointerKind kind = UiProto.PointerKind.fromCode(b.get() & 0xFF, "kind");
                    int x = b.getInt(), y = b.getInt();
                    int buttons = b.get() & 0xFF;
                    UiProto.Button button = UiProto.Button.fromCode(b.get() & 0xFF, "button");
                    int scroll = b.getInt();
                    UiConnection c = conns.get(b.getInt());
                    int hit = b.getInt();
                    drain(c);
                    comp.pointer(kind, x * hk.pointerScale(), y * hk.pointerScale(), buttons, button, scroll);
                    List<UiProto.Pointer> got = new ArrayList<>();
                    for (UiProto.Message m : drain(c)) if (m instanceof UiProto.Pointer p) got.add(p);
                    assertTrue(!got.isEmpty(), where + ": no Pointer event reached the app at " + x + "," + y);
                    for (UiProto.Pointer p : got) {
                        assertEquals(hit, p.hitId(), where + ": hit id at " + x + "," + y + " (" + p.kind() + ")");
                    }
                }
                case 9 -> conns.get(b.getInt()).close();
                default -> fail(where + ": unknown record type " + type);
            }
        }
        for (Map.Entry<Integer, UiConnection> e : conns.entrySet()) {
            UiError r = e.getValue().closeReason();
            assertTrue(r == null, name + ": connection " + e.getKey() + " was closed: " + (r == null ? "" : r.code + " " + r.getMessage()));
        }
        if (!failures.isEmpty()) fail(String.join("\n", failures));
        return comp;
    }

    private static String reason(UiConnection c) {
        UiError e = c.closeReason();
        return e == null ? "(no reason)" : e.code.wireName + ": " + e.getMessage();
    }

    private static byte[] bytes(ByteBuffer b) {
        byte[] out = new byte[b.getInt()];
        b.get(out);
        return out;
    }

    /** Messages queued for a connection, decoded. */
    static List<UiProto.Message> drain(UiConnection c) throws Exception {
        List<UiProto.Message> out = new ArrayList<>();
        while (true) {
            byte[] got = c.recv(1 << 22, 0);
            if (got == null || got.length == 0) return out;
            out.addAll(UiCodec.decodeAll(got, 0, got.length));
        }
    }

    /** Null if {@code rgba} equals the golden PNG; otherwise why (and the actual image is written for inspection). */
    static String compare(String name, byte[] rgba, int w, int h) throws IOException {
        BufferedImage actual = toImage(rgba, w, h);
        byte[] golden = resource("/ui-golden/" + name + ".png");
        Path root = repoRoot();
        if (golden == null || UPDATE) {
            Path dir = root.resolve("src/test/resources/ui-golden");
            Files.createDirectories(dir);
            ImageIO.write(actual, "png", dir.resolve(name + ".png").toFile());
            return UPDATE ? null : "golden " + name + ".png was missing; wrote it to " + dir + " (inspect it, then commit)";
        }
        BufferedImage exp = ImageIO.read(new ByteArrayInputStream(golden));
        if (exp.getWidth() != w || exp.getHeight() != h) return name + ": golden is " + exp.getWidth() + "x" + exp.getHeight() + ", frame " + w + "x" + h;
        int diff = 0, firstX = -1, firstY = -1;
        BufferedImage marked = toImage(rgba, w, h);
        for (int y = 0; y < h; y++) {
            for (int x = 0; x < w; x++) {
                if ((exp.getRGB(x, y) | 0xFF000000) != (actual.getRGB(x, y) | 0xFF000000)) {
                    if (diff++ == 0) {
                        firstX = x;
                        firstY = y;
                    }
                    marked.setRGB(x, y, 0xFFFF00FF);
                }
            }
        }
        if (diff == 0) return null;
        Path out = root.resolve("build/ui-golden-diff");
        Files.createDirectories(out);
        ImageIO.write(actual, "png", out.resolve(name + ".actual.png").toFile());
        ImageIO.write(marked, "png", out.resolve(name + ".diff.png").toFile());
        return name + ": " + diff + " pixels differ (first at " + firstX + "," + firstY + "); see " + out;
    }

    public static BufferedImage toImage(byte[] rgba, int w, int h) {
        BufferedImage img = new BufferedImage(w, h, BufferedImage.TYPE_INT_RGB);
        for (int y = 0; y < h; y++) {
            for (int x = 0; x < w; x++) {
                int i = (y * w + x) * 4;
                img.setRGB(x, y, (rgba[i] & 0xFF) << 16 | (rgba[i + 1] & 0xFF) << 8 | (rgba[i + 2] & 0xFF));
            }
        }
        return img;
    }
}
