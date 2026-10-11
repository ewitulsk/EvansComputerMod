package com.example.evanscomputermod.radio.wifi80211;

import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;

/** Hex helpers and fixture lookup for the 802.11 tests. */
final class Hex {

    private Hex() {}

    static byte[] bytes(String hex) {
        String s = hex.replaceAll("[\\s:]", "");
        if (s.length() % 2 != 0) throw new IllegalArgumentException("odd hex length");
        byte[] out = new byte[s.length() / 2];
        for (int i = 0; i < out.length; i++) out[i] = (byte) Integer.parseInt(s.substring(2 * i, 2 * i + 2), 16);
        return out;
    }

    static String of(byte[] b) {
        StringBuilder sb = new StringBuilder(b.length * 2);
        for (byte x : b) sb.append(String.format("%02x", x & 0xFF));
        return sb.toString();
    }

    /** Golden frame from src/test/resources/radio/frames: hex with '#' comment lines. */
    static byte[] resource(String name) {
        try (InputStream in = Hex.class.getResourceAsStream("/radio/frames/" + name)) {
            if (in == null) throw new IllegalStateException("missing fixture radio/frames/" + name);
            StringBuilder sb = new StringBuilder();
            for (String line : new String(in.readAllBytes(), StandardCharsets.UTF_8).split("\n")) {
                int hash = line.indexOf('#');
                sb.append(hash >= 0 ? line.substring(0, hash) : line).append(' ');
            }
            return bytes(sb.toString());
        } catch (IOException e) {
            throw new IllegalStateException(e);
        }
    }

    /** docs/radio/vectors, found by walking up from the working directory (Gradle runs tests in versions/<mc>). */
    static Path vectorsDir() {
        Path p = Path.of("").toAbsolutePath();
        while (p != null) {
            Path v = p.resolve("docs").resolve("radio").resolve("vectors");
            if (Files.isDirectory(v)) return v;
            p = p.getParent();
        }
        throw new IllegalStateException("docs/radio/vectors not found above " + Path.of("").toAbsolutePath());
    }
}
