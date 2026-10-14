package com.example.evanscomputermod.computer.ui.proto;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

import static org.junit.jupiter.api.Assertions.*;

/**
 * The shared UI protocol vectors (abi/ui-test-vectors/): every valid vector decodes to its canonical JSON and
 * re-encodes to the same bytes; every invalid one is rejected. The Rust codec runs the same files
 * (rust/crates/ecm-ui-proto/tests/vectors.rs), so the two codecs can't drift.
 */
class UiProtoVectorsTest {
    private static Path repoRoot() {
        Path p = Path.of("").toAbsolutePath();
        while (p != null && !Files.exists(p.resolve("abi/ui-protocol.toml"))) p = p.getParent();
        assertNotNull(p, "repository root (abi/ui-protocol.toml) not found");
        return p;
    }

    private static List<String> lines(String file) throws IOException {
        return Files.readAllLines(repoRoot().resolve("abi/ui-test-vectors").resolve(file), StandardCharsets.UTF_8)
                .stream().map(String::stripTrailing).filter(l -> !l.isEmpty() && !l.startsWith("#")).toList();
    }

    private static byte[] unhex(String s) {
        byte[] b = new byte[s.length() / 2];
        for (int i = 0; i < b.length; i++) b[i] = (byte) Integer.parseInt(s.substring(2 * i, 2 * i + 2), 16);
        return b;
    }

    @Test
    void everyValidVectorRoundTrips() throws Exception {
        Set<String> names = new HashSet<>();
        List<String> lines = lines("valid.txt");
        for (String line : lines) {
            String[] parts = line.split(" ", 3);
            byte[] bytes = unhex(parts[1]);
            UiProto.Message m;
            try {
                m = UiCodec.decodeExact(bytes);
            } catch (UiProtoException e) {
                throw new AssertionError(parts[0] + ": " + e.getMessage(), e);
            }
            assertEquals(parts[2], UiCodec.toJson(m), parts[0] + ": JSON differs");
            assertArrayEquals(bytes, UiCodec.encode(m), parts[0] + ": re-encoding differs");
            names.add(m.name());
        }
        assertTrue(lines.size() >= 100, "only " + lines.size() + " vectors");
        // Every message in the contract has a vector.
        for (Class<?> c : UiProto.Message.class.getPermittedSubclasses()) {
            assertTrue(names.contains(c.getSimpleName()), "no vector for " + c.getSimpleName());
        }
    }

    @Test
    void everyInvalidVectorIsRejected() throws Exception {
        List<String> lines = lines("invalid.txt");
        for (String line : lines) {
            String[] parts = line.split(" ", 2);
            try {
                UiProto.Message m = UiCodec.decodeExact(unhex(parts[1]));
                fail(parts[0] + ": decoded as " + UiCodec.toJson(m));
            } catch (UiProtoException expected) {
                // rejected
            }
        }
        assertTrue(lines.size() >= 30, "only " + lines.size() + " invalid vectors");
    }

    @Test
    void streamOfMessagesDecodesInOrder() throws Exception {
        var a = new UiProto.Commit(3);
        var b = new UiProto.Text(3, "héllo 😀");
        byte[] all = UiCodec.encodeAll(List.of(a, b));
        assertEquals(List.of(a, b), UiCodec.decodeAll(all, 0, all.length));
        UiProtoException e = assertThrows(UiProtoException.class, () -> UiCodec.decode(all, 0, 10));
        assertEquals(UiProtoException.Kind.INCOMPLETE, e.kind());
    }

    @Test
    void unknownOpcodeAndOversizeMapToProtocolErrors() {
        byte[] unknown = {0x77, 0x77, 0, 0, 0, 0, 0, 0};
        assertEquals(UiProto.ErrorCode.UNKNOWN_OPCODE,
                assertThrows(UiProtoException.class, () -> UiCodec.decodeExact(unknown)).errorCode());
        byte[] big = {0x01, 0x06, 0, 0, 1, 0, 0x10, 0};
        assertEquals(UiProto.ErrorCode.LIMIT,
                assertThrows(UiProtoException.class, () -> UiCodec.decode(big, 0, big.length)).errorCode());
    }
}
