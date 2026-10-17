package com.example.evanscomputermod.computer.upload;

import com.example.evanscomputermod.api.upload.UploadBackend;
import com.example.evanscomputermod.api.upload.UploadBackends;
import com.example.evanscomputermod.api.upload.UploadSink;
import com.example.evanscomputermod.api.upload.UploadStatus;
import net.minecraft.server.level.ServerPlayer;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

import static org.junit.jupiter.api.Assertions.*;

/**
 * The computer side of an upload ({@link UploadPort}): where the file lands,
 * the limits, and what a program polls. The player side (picker, network) is
 * the sidecar's, covered by the {@code upload_*} scenarios and the
 * {@code upload} client check.
 */
class UploadPortTest {
    @TempDir
    Path root;

    /** A backend with limits; it records cancels. */
    static final class Limits implements UploadBackend {
        long maxFile = 1000, maxComputer = 0;
        final List<Integer> cancelled = new ArrayList<>();

        public int apiVersion() { return CURRENT_API_VERSION; }
        public String name() { return "test"; }
        public boolean canUpload(ServerPlayer p) { return true; }
        public boolean permitted(ServerPlayer p) { return true; }
        public long maxFileBytes() { return maxFile; }
        public long maxComputerBytes() { return maxComputer; }
        public void start(ServerPlayer p, UploadSink s) {}
        public void cancel(UploadSink s) { cancelled.add(s.ticket()); }
    }

    final Limits backend = new Limits();

    UploadPort port() throws Exception {
        Files.createDirectories(root.resolve("home"));
        return new UploadPort(() -> root, () -> null, () -> "Terminal");
    }

    @AfterEach
    void reset() {
        UploadBackends.setForTesting(null);
    }

    static byte[] utf8(String s) {
        return s.getBytes(StandardCharsets.UTF_8);
    }

    @Test
    void aFileArrivesWholeUnderItsOwnName() throws Exception {
        UploadPort port = port();
        var s = port.openForTest(5, "/home", backend);
        assertEquals(UploadStatus.PICKING, port.poll(s.ticket(), 5).state());
        assertEquals(UploadStatus.OK, s.begin("C:\\Users\\me\\Desktop\\holiday.txt", 11));
        assertEquals(UploadStatus.OK, s.write(utf8("hello "), 0, 6));
        var mid = port.poll(s.ticket(), 5);
        assertEquals(UploadStatus.RECEIVING, mid.state());
        assertEquals(6, mid.received());
        assertEquals(11, mid.total());
        // Half a file is never visible under its name.
        assertFalse(Files.exists(root.resolve("home/holiday.txt")));
        assertEquals(UploadStatus.OK, s.write(utf8("world"), 0, 5));
        assertEquals(UploadStatus.OK, s.finish());
        var done = port.poll(s.ticket(), 5);
        assertEquals(UploadStatus.DONE, done.state());
        assertEquals("/home/holiday.txt", done.path());
        assertEquals("hello world", Files.readString(root.resolve("home/holiday.txt")));
        try (var l = Files.list(root.resolve("home"))) {
            assertEquals(1, l.count(), "no .part file left");
        }
        // Another program can't read this one's upload.
        assertEquals(UploadStatus.UNKNOWN, port.poll(s.ticket(), 6).state());
    }

    @Test
    void aTakenNameGetsANumber() throws Exception {
        UploadPort port = port();
        Files.writeString(root.resolve("home/a.txt"), "first");
        var s = port.openForTest(1, "home", backend);
        s.begin("a.txt", 1);
        s.write(utf8("2"), 0, 1);
        s.finish();
        assertEquals("/home/a (1).txt", s.path());
        assertEquals("first", Files.readString(root.resolve("home/a.txt")));
    }

    @Test
    void limitsAreEnforced() throws Exception {
        UploadPort port = port();
        var big = port.openForTest(1, "/home", backend);
        assertEquals(UploadStatus.TOO_LARGE, big.begin("big.bin", 1001));
        assertEquals(UploadStatus.TOO_LARGE, port.poll(big.ticket(), 1).state());

        backend.maxComputer = 100;
        Files.write(root.resolve("home/old.bin"), new byte[80]);
        var full = port.openForTest(1, "/home", backend);
        assertEquals(UploadStatus.STORAGE_FULL, full.begin("new.bin", 30));

        // More bytes than announced: the upload fails and leaves nothing.
        backend.maxComputer = 0;
        var liar = port.openForTest(1, "/home", backend);
        assertEquals(UploadStatus.OK, liar.begin("liar.txt", 2));
        assertEquals(UploadStatus.FAILED, liar.write(utf8("abc"), 0, 3));
        // Fewer bytes than announced: finish fails.
        var shortOne = port.openForTest(1, "/home", backend);
        shortOne.begin("short.txt", 5);
        shortOne.write(utf8("ab"), 0, 2);
        assertEquals(UploadStatus.FAILED, shortOne.finish());
        try (var l = Files.list(root.resolve("home"))) {
            assertEquals(List.of("old.bin"), l.map(p -> p.getFileName().toString()).toList());
        }
    }

    @Test
    void theProgramExitingCancelsItsUpload() throws Exception {
        UploadBackends.setForTesting(backend);
        UploadPort port = port();
        var s = port.openForTest(9, "/home", backend);
        s.begin("x.txt", 4);
        s.write(utf8("ab"), 0, 2);
        port.onProcessExit(9);
        assertEquals(UploadStatus.CANCELLED, port.poll(s.ticket(), 9).state());
        assertTrue(s.closed());
        try (var l = Files.list(root.resolve("home"))) {
            assertEquals(0, l.count(), "the partial file is gone");
        }
        // Late bytes from the player's game are refused.
        assertEquals(UploadStatus.CANCELLED, s.write(utf8("cd"), 0, 2));
    }

    @Test
    void beginChecksTheDirectoryAndTheAddOn() throws Exception {
        UploadPort port = port();
        assertEquals(UploadPort.ERR_NOT_INSTALLED, port.begin(1, "/home"));
        assertEquals(UploadPort.SUPPORT_NONE, port.supported());
        UploadBackends.setForTesting(backend);
        assertEquals(UploadPort.SUPPORT_NO_CLIENT, port.supported(), "nobody has used the computer yet");
        assertEquals(UploadPort.ERR_INVALID, port.begin(1, "/nope"));
        assertEquals(UploadPort.ERR_INVALID, port.begin(1, "/home/../.."));
        assertEquals(UploadPort.ERR_INVALID, port.begin(1, "C:\\Windows"));
        // A real directory, but no server to ask a player on: the upload fails at once.
        int t = port.begin(1, "/home");
        assertTrue(t > 0);
        assertEquals(UploadStatus.NO_CLIENT, port.poll(t, 1).state());
    }

    @Test
    void namesAreMadeSafe() {
        assertEquals("photo.png", UploadPort.sanitizeName("/home/me/photo.png"));
        assertEquals("evil.sh", UploadPort.sanitizeName("..\\..\\.evil.sh"));
        assertEquals("a_b_c.txt", UploadPort.sanitizeName("a:b*c.txt"));
        assertEquals("upload", UploadPort.sanitizeName("..."));
        String longName = "x".repeat(100) + ".jpeg";
        String cut = UploadPort.sanitizeName(longName);
        assertEquals(UploadPort.MAX_NAME, cut.length());
        assertTrue(cut.endsWith(".jpeg"));
        assertEquals("", UploadPort.normalizeDir("/"));
        assertEquals("home/pics", UploadPort.normalizeDir("/home/pics/"));
        assertNull(UploadPort.normalizeDir("home//x"));
    }
}
