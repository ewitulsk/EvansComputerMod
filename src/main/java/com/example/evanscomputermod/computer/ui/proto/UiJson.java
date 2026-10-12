package com.example.evanscomputermod.computer.ui.proto;

/** Canonical JSON pieces used by {@link UiProto} message rendering (the shared test-vector format). */
public final class UiJson {
    private static final char[] HEX = "0123456789abcdef".toCharArray();

    private UiJson() {}

    /** A JSON string escaping only {@code "}, {@code \} and U+0000..U+001F (as {@code \}u00xx). */
    public static void str(StringBuilder o, String s) {
        o.append('"');
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            if (c == '"') o.append("\\\"");
            else if (c == '\\') o.append("\\\\");
            else if (c < 0x20) o.append("\\u00").append(HEX[c >> 4]).append(HEX[c & 15]);
            else o.append(c);
        }
        o.append('"');
    }

    public static void hex(StringBuilder o, byte[] b) {
        o.append('"');
        for (byte x : b) o.append(HEX[(x >> 4) & 15]).append(HEX[x & 15]);
        o.append('"');
    }
}
