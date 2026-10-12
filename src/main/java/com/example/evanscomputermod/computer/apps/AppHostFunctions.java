package com.example.evanscomputermod.computer.apps;

import com.example.evanscomputermod.api.wasm.WasmHostFunc;
import com.example.evanscomputermod.api.wasm.WasmMemory;
import com.example.evanscomputermod.api.wasm.WasmValType;
import com.example.evanscomputermod.computer.wasi.WasiFunctions;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;
import java.util.Map;

/**
 * The {@code app_ctl} child import (abi/child-abi.toml {@code [imports.apps]},
 * ecm-host-abi {@code apps.rs}): one text request to the app manager per call.
 *
 * <p>{@code app_ctl(req, len, reply, cap, status_out) -> full reply length}
 * (only {@code min(len, cap)} bytes are written; {@code output} never hands
 * over more than {@code cap}). Words are separated by {@code '\n'}. Status 0 =
 * ok, -2 not found, -13 not permitted, -22 bad request, -35 choose an app.
 * Confined apps may only use {@code whoami} unless they hold {@code packages}.
 */
final class AppHostFunctions {

    private static final List<WasmValType> I32x5 = List.of(WasmValType.I32, WasmValType.I32, WasmValType.I32,
            WasmValType.I32, WasmValType.I32);
    private static final List<WasmValType> RET_I32 = List.of(WasmValType.I32);

    record Reply(int status, byte[] data) {
        static Reply ok(String s) {
            return new Reply(0, s.getBytes(StandardCharsets.UTF_8));
        }

        static Reply err(int status, String s) {
            return new Reply(status, s.getBytes(StandardCharsets.UTF_8));
        }
    }

    private AppHostFunctions() {}

    static void register(AppManager mgr, WasiFunctions.WasiState state, List<WasmHostFunc> sink, int pid) {
        WasmHostFunc.Handler h = (inst, args) -> {
            WasmMemory mem = state.instance.memory();
            int len = Math.max(0, Math.min((int) args[1], 8192));
            String req = new String(mem.readBytes((int) args[0], len), StandardCharsets.UTF_8);
            int cap = Math.max(0, (int) args[3]);
            Reply r = control(mgr, pid, Arrays.asList(req.split("\n", -1)), cap);
            int n = Math.min(r.data().length, cap);
            if (n > 0) mem.writeBytes((int) args[2], r.data(), 0, n);
            mem.writeInt((int) args[4], r.status());
            return WasmHostFunc.retI32(r.data().length);
        };
        sink.add(new WasmHostFunc("env", "app_ctl", I32x5, RET_I32, h));
        sink.add(new WasmHostFunc("", "app_ctl", I32x5, RET_I32, h));
    }

    static Reply control(AppManager mgr, int pid, List<String> w, int cap) {
        String cmd = w.isEmpty() ? "" : w.get(0);
        AppInstance caller = mgr.instanceOf(pid);
        if (cmd.equals("whoami")) return Reply.ok(caller == null ? "" : caller.appId());
        if (caller != null && !mgr.check(pid, "packages")) {
            return Reply.err(AppException.DENIED, "apps need the packages permission for app_ctl " + cmd);
        }
        try {
            return switch (cmd) {
                case "reindex" -> {
                    int n = mgr.reindex();
                    StringBuilder sb = new StringBuilder(n + " apps\n");
                    mgr.brokenApps().forEach((f, why) -> sb.append("broken /apps/").append(f).append(": ").append(why).append('\n'));
                    yield Reply.ok(sb.toString());
                }
                case "list" -> {
                    StringBuilder sb = new StringBuilder();
                    for (AppManifest m : mgr.installedApps().values()) {
                        sb.append(m.id()).append('\t').append(m.version()).append('\t').append(m.name()).append('\n');
                    }
                    yield Reply.ok(sb.toString());
                }
                case "ps" -> {
                    StringBuilder sb = new StringBuilder();
                    for (AppInstance i : mgr.running()) {
                        sb.append(i.pid()).append('\t').append(i.appId()).append('\t').append(i.state()).append('\n');
                    }
                    yield Reply.ok(sb.toString());
                }
                case "launch" -> {
                    if (w.size() < 2) yield Reply.err(AppException.INVALID, "launch <id> [args]");
                    AppInstance i = mgr.launch(w.get(1), new ArrayList<>(w.subList(2, w.size())));
                    yield Reply.ok(Integer.toString(i.pid()));
                }
                case "status" -> {
                    AppInstance i = byPid(mgr, w);
                    if (i == null) yield Reply.err(AppException.NOT_FOUND, "no such app process");
                    StringBuilder sb = new StringBuilder();
                    Integer code = i.exitCode();
                    sb.append(code != null ? "exited " + code : i.state().name().toLowerCase(java.util.Locale.ROOT)).append('\n');
                    for (String d : i.denials()) sb.append("denied ").append(d).append('\n');
                    yield Reply.ok(sb.toString());
                }
                case "output" -> {
                    AppInstance i = byPid(mgr, w);
                    if (i == null) yield Reply.err(AppException.NOT_FOUND, "no such app process");
                    yield new Reply(0, i.takeOutput(cap));
                }
                case "kill" -> {
                    AppInstance i = byPid(mgr, w);
                    if (i == null) yield Reply.err(AppException.NOT_FOUND, "no such app process");
                    mgr.kill(i);
                    yield Reply.ok("");
                }
                case "perm" -> perm(mgr, w);
                case "open" -> {
                    if (w.size() != 2) yield Reply.err(AppException.INVALID, "open <path>");
                    AppInstance i = mgr.open(w.get(1), caller != null ? pid : 0);
                    yield Reply.ok(i.appId() + " " + i.pid());
                }
                case "default" -> {
                    if (w.size() != 3 || !w.get(1).startsWith(".")) yield Reply.err(AppException.INVALID, "default <.ext> <id|->");
                    mgr.setDefaultHandler(w.get(1), w.get(2).equals("-") ? null : w.get(2));
                    yield Reply.ok("");
                }
                case "session" -> Reply.ok(String.join("\n", mgr.lastSession()));
                default -> Reply.err(AppException.INVALID, "unknown request '" + cmd + "'");
            };
        } catch (AppException e) {
            return Reply.err(e.code, e.getMessage());
        }
    }

    private static AppInstance byPid(AppManager mgr, List<String> w) {
        if (w.size() < 2) return null;
        try {
            return mgr.byRootPid(Integer.parseInt(w.get(1).trim()));
        } catch (NumberFormatException e) {
            return null;
        }
    }

    private static Reply perm(AppManager mgr, List<String> w) {
        String op = w.size() > 1 ? w.get(1) : "list";
        PermissionStore store = mgr.permissions();
        if (op.equals("list")) {
            String only = w.size() > 2 ? w.get(2) : null;
            StringBuilder sb = new StringBuilder();
            for (Map.Entry<String, Map<String, Boolean>> e : store.decisions().entrySet()) {
                if (only != null && !only.equals(e.getKey())) continue;
                e.getValue().forEach((p, a) -> sb.append(e.getKey()).append(' ').append(p).append(' ').append(a ? "allow" : "deny").append('\n'));
            }
            for (String id : mgr.installedApps().keySet()) {
                if (only != null && !only.equals(id)) continue;
                store.grants(id).forEach((path, rw) -> sb.append(id).append(" file ").append(path).append(' ').append(rw ? "rw" : "r").append('\n'));
                if (only != null) {
                    for (String p : mgr.manifest(id).permissions()) {
                        if (store.decision(id, p) == null) sb.append(id).append(' ').append(p).append(" ask\n");
                    }
                }
            }
            return Reply.ok(sb.toString());
        }
        if (w.size() != 4) return Reply.err(AppException.INVALID, "perm grant|deny|reset <id> <permission>");
        String id = w.get(2), p = w.get(3);
        if (!AppManifest.validId(id) || !AppManifest.validPermission(p)) {
            return Reply.err(AppException.INVALID, "bad app id or permission");
        }
        switch (op) {
            case "grant", "deny" -> {
                AppManifest m = mgr.manifest(id);
                if (m == null) return Reply.err(AppException.NOT_FOUND, id + " is not installed");
                if (!m.permissions().contains(p)) return Reply.err(AppException.INVALID, id + " does not ask for " + p);
                store.setDecision(id, p, op.equals("grant"));
                return Reply.ok(id + " " + p + " " + (op.equals("grant") ? "allow" : "deny"));
            }
            case "reset" -> {
                store.reset(id, p);
                return Reply.ok(id + " " + p + " ask");
            }
            default -> {
                return Reply.err(AppException.INVALID, "perm list|grant|deny|reset");
            }
        }
    }
}
