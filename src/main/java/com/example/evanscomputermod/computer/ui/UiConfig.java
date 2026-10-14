package com.example.evanscomputermod.computer.ui;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

/**
 * {@code /etc/ui.toml} (§2): whether the GUI starts at boot, which shell and
 * theme it uses, and what an attached Screen cluster shows.
 *
 * <pre>
 * autostart = false
 * shell     = "dev.ecm.desktop"     # a program name (or app id) with the shell role
 * theme     = "ecm-default"
 * [screen]
 * mode      = "mirror"              # "mirror" | "own" | "off"
 * shell     = "dev.ecm.kiosk"       # used when mode = "own"
 * </pre>
 *
 * Only this small TOML subset is read: {@code key = value} lines with
 * strings, booleans or string arrays, one {@code [screen]} table, comments.
 * Unknown keys are ignored; a missing file means the defaults.
 */
public record UiConfig(boolean autostart, String shell, List<String> shellArgs, String theme, String screenMode,
                       String screenShell) {
    /** The desktop shell (ROADMAP P3); {@code test-shell} is the P2 tiling shell. */
    public static final String DEFAULT_SHELL = "dev.ecm.desktop";

    public static UiConfig defaults() {
        return new UiConfig(false, DEFAULT_SHELL, List.of(), "ecm-default", "mirror", "");
    }

    public static UiConfig load(Path file) {
        if (file == null || !Files.isRegularFile(file)) return defaults();
        try {
            return parse(Files.readString(file, StandardCharsets.UTF_8));
        } catch (IOException e) {
            return defaults();
        }
    }

    public static UiConfig parse(String text) {
        boolean autostart = false;
        String shell = DEFAULT_SHELL, theme = "ecm-default", mode = "mirror", screenShell = "";
        List<String> args = List.of();
        String table = "";
        for (String raw : text.split("\n")) {
            String line = stripComment(raw).trim();
            if (line.isEmpty()) continue;
            if (line.startsWith("[") && line.endsWith("]")) {
                table = line.substring(1, line.length() - 1).trim();
                continue;
            }
            int eq = line.indexOf('=');
            if (eq < 0) continue;
            String key = line.substring(0, eq).trim();
            String value = line.substring(eq + 1).trim();
            if (table.isEmpty()) {
                switch (key) {
                    case "autostart" -> autostart = value.equals("true");
                    case "shell" -> shell = string(value, shell);
                    case "shell_args" -> args = array(value);
                    case "theme" -> theme = string(value, theme);
                    default -> { }
                }
            } else if (table.equals("screen")) {
                switch (key) {
                    case "mode" -> mode = string(value, mode);
                    case "shell" -> screenShell = string(value, screenShell);
                    default -> { }
                }
            }
        }
        if (!List.of("mirror", "own", "off").contains(mode)) mode = "mirror";
        return new UiConfig(autostart, shell, args, theme, mode, screenShell);
    }

    private static String stripComment(String s) {
        boolean inStr = false;
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            if (c == '"') inStr = !inStr;
            else if (c == '#' && !inStr) return s.substring(0, i);
        }
        return s;
    }

    private static String string(String v, String fallback) {
        if (v.length() >= 2 && v.startsWith("\"") && v.endsWith("\"")) return v.substring(1, v.length() - 1);
        return v.isEmpty() ? fallback : v;
    }

    private static List<String> array(String v) {
        List<String> out = new ArrayList<>();
        if (!v.startsWith("[") || !v.endsWith("]")) return out;
        for (String part : v.substring(1, v.length() - 1).split(",")) {
            String p = part.trim();
            if (!p.isEmpty()) out.add(string(p, p));
        }
        return out;
    }
}
