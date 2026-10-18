package com.example.evanscomputermod.computer.apps;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.Set;

/**
 * An app's {@code app.toml} (docs/gui/APPS.md, "app.toml"). The same schema
 * and checks as {@code rust/wasm-programs/pkg/src/manifest.rs}; both run the
 * vectors in {@code abi/app-manifest-vectors}.
 *
 * @param permissions every permission the app may ever be granted: the
 *                    {@code permissions} list plus {@code background:<mode>}
 *                    for each background wish
 */
public record AppManifest(
        String id,
        String name,
        String version,
        String entry,
        String summary,
        String publisher,
        List<String> sizes,
        List<String> permissions,
        boolean backgroundRefresh,
        List<String> backgroundModes,
        List<String> openWith,
        List<Hotkey> hotkeys,
        String icon) {

    public record Hotkey(int slot, String action, String label) {}

    /** A manifest problem; {@link #field} names the key ({@code syntax}, {@code unknown} or a field). */
    public static final class ManifestException extends Exception {
        public final String field;

        public ManifestException(String field, String message) {
            super("app.toml: " + field + ": " + message);
            this.field = field;
        }
    }

    public static final int MAX_ID = 128;
    public static final List<String> SIZES = List.of("compact", "medium", "wide");
    public static final List<String> BACKGROUND_MODES = List.of("audio", "call", "radio", "transfer", "navigation");
    /** Plain permission names (besides {@code peripherals:<type>} and {@code background:<mode>}). */
    public static final List<String> PERMISSIONS = List.of(
            "network", "radio", "cellular", "controller", "microphone", "location", "notifications",
            "files:documents", "files:all", "packages", "mixer", "contacts", "phone", "sms");
    private static final Set<String> KNOWN_KEYS = Set.of(
            "id", "name", "version", "entry", "summary", "publisher", "sizes", "permissions", "background",
            "open_with", "hotkeys", "icon");

    public boolean isPython() {
        return entry.endsWith(".py");
    }

    public static boolean validPermission(String p) {
        if (PERMISSIONS.contains(p)) return true;
        if (p.startsWith("peripherals:")) {
            String t = p.substring("peripherals:".length());
            return !t.isEmpty() && t.length() <= 32 && t.chars().allMatch(c -> (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '_');
        }
        if (p.startsWith("background:")) {
            String m = p.substring("background:".length());
            return m.equals("refresh") || BACKGROUND_MODES.contains(m);
        }
        return false;
    }

    public static boolean validId(String id) {
        if (id == null || id.isEmpty() || id.length() > MAX_ID) return false;
        String[] labels = id.split("\\.", -1);
        if (labels.length < 2) return false;
        for (int i = 0; i < labels.length; i++) {
            String l = labels[i];
            if (l.isEmpty() || !l.chars().allMatch(c -> (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '_' || c == '-')) return false;
            if (i == 0 && !(l.charAt(0) >= 'a' && l.charAt(0) <= 'z')) return false;
        }
        return true;
    }

    /** {@code MAJOR.MINOR.PATCH}, optionally followed by {@code -pre} or {@code +build}. */
    public static boolean validVersion(String v) {
        return parseVersion(v) != null;
    }

    private record Version(long[] core, String rest) {}

    private static Version parseVersion(String v) {
        if (v == null || v.isEmpty() || v.length() > 32) return null;
        int cut = -1;
        for (int i = 0; i < v.length(); i++) {
            if (v.charAt(i) == '-' || v.charAt(i) == '+') {
                cut = i;
                break;
            }
        }
        String core = cut < 0 ? v : v.substring(0, cut);
        String rest = cut < 0 ? "" : v.substring(cut);
        if (!rest.isEmpty() && (rest.length() < 2 || !rest.substring(1).chars()
                .allMatch(c -> Character.isLetterOrDigit(c) && c < 128 || c == '.' || c == '-'))) return null;
        String[] parts = core.split("\\.", -1);
        if (parts.length != 3) return null;
        long[] n = new long[3];
        for (int i = 0; i < 3; i++) {
            String p = parts[i];
            if (p.isEmpty() || p.length() > 9 || !p.chars().allMatch(c -> c >= '0' && c <= '9')) return null;
            n[i] = Long.parseLong(p);
        }
        return new Version(n, rest);
    }

    /** Numeric MAJOR.MINOR.PATCH, then a pre-release sorts before the release; invalid sorts first. */
    public static int compareVersions(String a, String b) {
        Version va = parseVersion(a), vb = parseVersion(b);
        if (va == null || vb == null) {
            if (va == null && vb == null) return a.compareTo(b);
            return va == null ? -1 : 1;
        }
        for (int i = 0; i < 3; i++) {
            int c = Long.compare(va.core[i], vb.core[i]);
            if (c != 0) return c;
        }
        boolean pa = va.rest.startsWith("-"), pb = vb.rest.startsWith("-");
        if (pa != pb) return pa ? -1 : 1;
        return Integer.signum(va.rest.compareTo(vb.rest));
    }

    /** A safe relative path: no leading '/', no '..', '.', '\' or empty parts, no control characters. */
    public static boolean safeRelPath(String p) {
        if (p == null || p.isEmpty() || p.length() > 255 || p.startsWith("/") || p.contains("\\")) return false;
        for (String part : p.split("/", -1)) {
            if (part.isEmpty() || part.equals("..") || part.equals(".")) return false;
        }
        return p.chars().noneMatch(c -> c < 0x20);
    }

    public static AppManifest parse(String text) throws ManifestException {
        Map<String, Object> t;
        try {
            t = MiniToml.parse(text);
        } catch (MiniToml.SyntaxException e) {
            throw new ManifestException("syntax", e.getMessage());
        }
        return fromTable(t);
    }

    private static String text(String field, Object v, int max, boolean nonEmpty) throws ManifestException {
        if (!(v instanceof String s)) throw new ManifestException(field, "must be a string, not " + typeName(v));
        if (nonEmpty && s.isBlank()) throw new ManifestException(field, "must not be empty");
        if (s.codePointCount(0, s.length()) > max) throw new ManifestException(field, "longer than " + max + " characters");
        if (s.codePoints().anyMatch(Character::isISOControl)) throw new ManifestException(field, "contains control characters");
        return s;
    }

    private static String typeName(Object v) {
        if (v instanceof String) return "string";
        if (v instanceof Long) return "integer";
        if (v instanceof Boolean) return "boolean";
        if (v instanceof List) return "array";
        return "table";
    }

    private static List<String> strList(String field, Object v) throws ManifestException {
        if (!(v instanceof List<?> items)) throw new ManifestException(field, "must be an array of strings");
        List<String> out = new ArrayList<>();
        for (Object it : items) {
            if (!(it instanceof String s)) throw new ManifestException(field, "must be an array of strings");
            if (out.contains(s)) throw new ManifestException(field, "'" + s + "' is listed twice");
            out.add(s);
        }
        return out;
    }

    @SuppressWarnings("unchecked")
    public static AppManifest fromTable(Map<String, Object> t) throws ManifestException {
        for (String k : t.keySet()) {
            if (!KNOWN_KEYS.contains(k)) throw new ManifestException("unknown", "unknown key '" + k + "'");
        }
        if (!t.containsKey("id")) throw new ManifestException("id", "missing");
        String id = text("id", t.get("id"), MAX_ID, true);
        if (!validId(id)) throw new ManifestException("id", "'" + id + "' is not a reverse-DNS id like dev.you.app");
        if (!t.containsKey("name")) throw new ManifestException("name", "missing");
        String name = text("name", t.get("name"), 64, true);
        if (!t.containsKey("version")) throw new ManifestException("version", "missing");
        String version = text("version", t.get("version"), 32, true);
        if (!validVersion(version)) throw new ManifestException("version", "'" + version + "' is not MAJOR.MINOR.PATCH");
        if (!t.containsKey("entry")) throw new ManifestException("entry", "missing");
        String entry = text("entry", t.get("entry"), 255, true);
        if (!safeRelPath(entry) || !(entry.endsWith(".wasm") || entry.endsWith(".py"))) {
            throw new ManifestException("entry", "'" + entry + "' must be a relative path to a .wasm or .py file");
        }
        String summary = t.containsKey("summary") ? text("summary", t.get("summary"), 256, false) : "";
        String publisher = t.containsKey("publisher") ? text("publisher", t.get("publisher"), 64, false) : "";
        List<String> sizes;
        if (t.containsKey("sizes")) {
            sizes = strList("sizes", t.get("sizes"));
            if (sizes.isEmpty()) throw new ManifestException("sizes", "must list at least one size");
            for (String s : sizes) {
                if (!SIZES.contains(s)) throw new ManifestException("sizes", "unknown size '" + s + "' (compact, medium, wide)");
            }
        } else {
            sizes = SIZES;
        }
        List<String> permissions = t.containsKey("permissions") ? strList("permissions", t.get("permissions")) : new ArrayList<>();
        for (String p : permissions) {
            if (!validPermission(p)) throw new ManifestException("permissions", "unknown permission '" + p + "'");
        }
        boolean refresh = false;
        List<String> modes = new ArrayList<>();
        if (t.containsKey("background")) {
            if (!(t.get("background") instanceof Map<?, ?> bgRaw)) {
                throw new ManifestException("background", "must be a table like { refresh = true }");
            }
            Map<String, Object> bg = (Map<String, Object>) bgRaw;
            for (String k : bg.keySet()) {
                if (!k.equals("refresh") && !k.equals("modes")) throw new ManifestException("background", "unknown key '" + k + "'");
            }
            if (bg.containsKey("refresh")) {
                if (!(bg.get("refresh") instanceof Boolean b)) throw new ManifestException("background", "refresh must be true or false");
                refresh = b;
            }
            if (bg.containsKey("modes")) {
                modes = strList("background", bg.get("modes"));
                for (String m : modes) {
                    if (!BACKGROUND_MODES.contains(m)) {
                        throw new ManifestException("background", "unknown mode '" + m + "' (audio, call, radio, transfer, navigation)");
                    }
                }
            }
        }
        if (refresh && !permissions.contains("background:refresh")) permissions.add("background:refresh");
        for (String m : modes) {
            if (!permissions.contains("background:" + m)) permissions.add("background:" + m);
        }
        List<String> openWith = new ArrayList<>();
        if (t.containsKey("open_with")) {
            openWith = strList("open_with", t.get("open_with"));
            for (String e : openWith) {
                boolean ok = e.length() >= 2 && e.length() <= 17 && e.startsWith(".")
                        && e.substring(1).chars().allMatch(c -> (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9'));
                if (!ok) throw new ManifestException("open_with", "'" + e + "' must look like \".txt\" (lower case)");
            }
        }
        List<Hotkey> hotkeys = new ArrayList<>();
        if (t.containsKey("hotkeys")) {
            if (!(t.get("hotkeys") instanceof List<?> items)) {
                throw new ManifestException("hotkeys", "must be an array of { slot, action, label }");
            }
            for (Object it : items) {
                if (!(it instanceof Map<?, ?> hRaw)) throw new ManifestException("hotkeys", "must be an array of { slot, action, label }");
                Map<String, Object> h = (Map<String, Object>) hRaw;
                for (String k : h.keySet()) {
                    if (!k.equals("slot") && !k.equals("action") && !k.equals("label")) {
                        throw new ManifestException("hotkeys", "unknown key '" + k + "'");
                    }
                }
                if (!(h.get("slot") instanceof Long slot) || slot < 1 || slot > 6) throw new ManifestException("hotkeys", "slot must be 1..6");
                for (Hotkey k : hotkeys) {
                    if (k.slot() == slot) throw new ManifestException("hotkeys", "slot " + slot + " is used twice");
                }
                if (!(h.get("action") instanceof String action) || action.isEmpty() || action.length() > 64
                        || !action.chars().allMatch(c -> (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '_')) {
                    throw new ManifestException("hotkeys", "action must be a name like new_note");
                }
                if (!h.containsKey("label")) throw new ManifestException("hotkeys", "label is missing");
                String label = text("hotkeys", h.get("label"), 32, true);
                hotkeys.add(new Hotkey(slot.intValue(), action, label));
            }
        }
        String icon = "";
        if (t.containsKey("icon")) {
            icon = text("icon", t.get("icon"), 255, true);
            if (!safeRelPath(icon) || !icon.endsWith(".png")) {
                throw new ManifestException("icon", "'" + icon + "' must be a relative path to a .png file");
            }
        }
        return new AppManifest(id, name, version, entry, summary, publisher, List.copyOf(sizes), List.copyOf(permissions),
                refresh, List.copyOf(modes), List.copyOf(openWith), List.copyOf(hotkeys), icon);
    }
}
