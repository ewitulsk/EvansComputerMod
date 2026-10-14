package com.example.evanscomputermod.computer.apps;

import com.example.evanscomputermod.EvansComputerMod;
import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import org.jetbrains.annotations.Nullable;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.TreeMap;

/**
 * A computer's app database, {@code /var/lib/apps/db.json} on its disk
 * (docs/gui/APPS.md, "Permission database"): remembered permission answers,
 * one-file grants (file picker, open with) and the "open with" defaults.
 * Confined apps can never reach {@code /var/lib/apps}. Every change is saved
 * at once; the file is small.
 *
 * <pre>
 * { "decisions": { "dev.ecm.hello": { "network": "allow", "radio": "deny" } },
 *   "grants":    { "dev.ecm.editor": { "/home/notes.txt": "rw" } },
 *   "defaults":  { ".txt": "dev.ecm.editor" } }
 * </pre>
 */
public final class PermissionStore {

    public static final String DB_DIR = "/var/lib/apps";
    private static final Gson GSON = new GsonBuilder().setPrettyPrinting().create();

    private final Path file;
    /** appId -> permission -> allow? */
    private final Map<String, Map<String, Boolean>> decisions = new TreeMap<>();
    /** appId -> virtual path -> writable? */
    private final Map<String, Map<String, Boolean>> grants = new TreeMap<>();
    /** ".ext" -> appId */
    private final Map<String, String> defaults = new TreeMap<>();

    public PermissionStore(Path storageRoot) {
        this.file = storageRoot.resolve("var/lib/apps/db.json");
        load();
    }

    private synchronized void load() {
        if (!Files.isRegularFile(file)) return;
        try {
            JsonObject o = JsonParser.parseString(Files.readString(file, StandardCharsets.UTF_8)).getAsJsonObject();
            readNested(o, "decisions", decisions, v -> v.equals("allow") ? Boolean.TRUE : v.equals("deny") ? Boolean.FALSE : null);
            readNested(o, "grants", grants, v -> v.equals("rw") ? Boolean.TRUE : v.equals("r") ? Boolean.FALSE : null);
            if (o.has("defaults") && o.get("defaults").isJsonObject()) {
                for (var e : o.getAsJsonObject("defaults").entrySet()) {
                    if (e.getValue().isJsonPrimitive()) defaults.put(e.getKey(), e.getValue().getAsString());
                }
            }
        } catch (Exception e) {
            EvansComputerMod.LOGGER.warn("[apps] unreadable {}; starting with no decisions: {}", file, e.toString());
        }
    }

    private static void readNested(JsonObject o, String key, Map<String, Map<String, Boolean>> into,
                                   java.util.function.Function<String, Boolean> conv) {
        if (!o.has(key) || !o.get(key).isJsonObject()) return;
        for (var app : o.getAsJsonObject(key).entrySet()) {
            if (!app.getValue().isJsonObject()) continue;
            Map<String, Boolean> m = new TreeMap<>();
            for (var e : app.getValue().getAsJsonObject().entrySet()) {
                if (!e.getValue().isJsonPrimitive()) continue;
                Boolean b = conv.apply(e.getValue().getAsString());
                if (b != null) m.put(e.getKey(), b);
            }
            if (!m.isEmpty()) into.put(app.getKey(), m);
        }
    }

    private synchronized void save() {
        Map<String, Object> root = new LinkedHashMap<>();
        Map<String, Map<String, String>> d = new TreeMap<>();
        decisions.forEach((app, m) -> {
            Map<String, String> x = new TreeMap<>();
            m.forEach((p, a) -> x.put(p, a ? "allow" : "deny"));
            d.put(app, x);
        });
        Map<String, Map<String, String>> g = new TreeMap<>();
        grants.forEach((app, m) -> {
            Map<String, String> x = new TreeMap<>();
            m.forEach((p, w) -> x.put(p, w ? "rw" : "r"));
            g.put(app, x);
        });
        root.put("decisions", d);
        root.put("grants", g);
        root.put("defaults", defaults);
        try {
            Files.createDirectories(file.getParent());
            Files.writeString(file, GSON.toJson(root) + "\n", StandardCharsets.UTF_8);
        } catch (IOException e) {
            EvansComputerMod.LOGGER.warn("[apps] cannot save {}: {}", file, e.toString());
        }
    }

    // ---------------------------------------------------------------- decisions

    /** The remembered answer: true allow, false deny, null not decided. */
    @Nullable
    public synchronized Boolean decision(String appId, String permission) {
        Map<String, Boolean> m = decisions.get(appId);
        return m == null ? null : m.get(permission);
    }

    public synchronized void setDecision(String appId, String permission, boolean allow) {
        decisions.computeIfAbsent(appId, k -> new TreeMap<>()).put(permission, allow);
        save();
    }

    /** Forget an answer (the app is asked again). Returns true if there was one. */
    public synchronized boolean reset(String appId, String permission) {
        Map<String, Boolean> m = decisions.get(appId);
        if (m == null || m.remove(permission) == null) return false;
        if (m.isEmpty()) decisions.remove(appId);
        save();
        return true;
    }

    /** Every remembered answer: appId -> permission -> allow (a copy, for Settings). */
    public synchronized Map<String, Map<String, Boolean>> decisions() {
        Map<String, Map<String, Boolean>> out = new TreeMap<>();
        decisions.forEach((k, v) -> out.put(k, new TreeMap<>(v)));
        return out;
    }

    /** Drop everything about an app (uninstall with purge). */
    public synchronized void forgetApp(String appId) {
        boolean changed = decisions.remove(appId) != null | grants.remove(appId) != null;
        changed |= defaults.values().removeIf(appId::equals);
        if (changed) save();
    }

    // ---------------------------------------------------------------- one-file grants

    /** Let {@code appId} open the one file {@code vpath} (read, or read-write). */
    public synchronized void grant(String appId, String vpath, boolean write) {
        Map<String, Boolean> m = grants.computeIfAbsent(appId, k -> new TreeMap<>());
        m.merge(vpath, write, Boolean::logicalOr);
        save();
    }

    /** null: no grant; false: read-only; true: read-write. */
    @Nullable
    public synchronized Boolean grantOf(String appId, String vpath) {
        Map<String, Boolean> m = grants.get(appId);
        return m == null ? null : m.get(vpath);
    }

    public synchronized boolean revokeGrant(String appId, String vpath) {
        Map<String, Boolean> m = grants.get(appId);
        if (m == null || m.remove(vpath) == null) return false;
        if (m.isEmpty()) grants.remove(appId);
        save();
        return true;
    }

    /** An app's grants: path -> writable (a copy). */
    public synchronized Map<String, Boolean> grants(String appId) {
        return new TreeMap<>(grants.getOrDefault(appId, Map.of()));
    }

    // ---------------------------------------------------------------- open-with defaults

    @Nullable
    public synchronized String defaultHandler(String ext) {
        return defaults.get(ext);
    }

    /** Set (or with null clear) the default app for {@code ".ext"}. */
    public synchronized void setDefaultHandler(String ext, @Nullable String appId) {
        if (appId == null) defaults.remove(ext);
        else defaults.put(ext, appId);
        save();
    }

    public synchronized Map<String, String> defaultHandlers() {
        return new TreeMap<>(defaults);
    }
}
