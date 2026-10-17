package com.example.evanscomputermod.computer.apps;

import com.example.evanscomputermod.EcmConfig;
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.computer.wasi.ProcessManager;
import com.example.evanscomputermod.computer.wasi.ProcessSandbox;
import org.jetbrains.annotations.Nullable;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collection;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeMap;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.function.Function;
import java.util.function.Supplier;

/**
 * One per computer: installed apps, running apps and their identity,
 * permissions, "open with", saved state and (on ePhones) background rules
 * (docs/gui/APPS.md). It is also the computer's {@link ProcessSandbox}: the
 * child host functions ask it before sockets, radios, peripherals and files.
 *
 * <p><b>Identity.</b> {@link #launch} starts an app as its own process group
 * (registered before the first instruction runs). {@link #appIdOf} maps any
 * pid to its app through its group. A program the kernel starts on an app's
 * behalf (a line the app typed into its shell session) is spawned into the
 * app's group ({@link #groupForSpawn}), so it has the app's permissions.
 * Programs typed in the Terminal are in no app group: unconfined (also in the
 * ePhone's built-in Terminal app, a system shell: {@link #isSystemShell}).
 *
 * <p><b>For the compositor (P2).</b> Install a {@link PermissionPrompter}
 * ({@link #setPrompter}) and an {@link AppEvents} ({@link #setAppEvents});
 * call {@link #appIdOf}, {@link #manifest}, {@link #launch}, {@link #open},
 * {@link #onSaveState}, {@link #onBackgroundDone}, {@link #onFilePicked},
 * {@link #peekRestore}/{@link #takeRestore}, {@link #check} (e.g.
 * {@code notifications}). <b>For the phone (P5).</b> {@link #setPolicy}
 * ({@link LifecyclePolicy#PHONE}), {@link #suspend}/{@link #resume},
 * {@link #background()}.
 */
public final class AppManager implements ProcessSandbox, AutoCloseable {

    /** Group ids of apps start here, far above any pid, so a group is never mistaken for a pid's own. */
    private static final int GROUP_BASE = 1 << 24;
    private static final AtomicInteger NEXT_GROUP = new AtomicInteger(GROUP_BASE);

    private static final ScheduledExecutorService IO = java.util.concurrent.Executors.newSingleThreadScheduledExecutor(r -> {
        Thread t = new Thread(r, "ECM-App-IO");
        t.setDaemon(true);
        return t;
    });

    private final Path root;
    private final ProcessManager pm;
    private final Function<String, Path> programResolver;
    private final Supplier<Map<String, String>> baseEnv;
    private final PermissionStore store;
    private final AppStateStore states;
    private final BackgroundScheduler background;

    private final Map<String, AppManifest> installed = new ConcurrentHashMap<>();
    private final Map<String, String> broken = new ConcurrentHashMap<>();
    /** group -> running app. */
    private final Map<Integer, AppInstance> instances = new ConcurrentHashMap<>();
    /** "appId\npermission" -> the prompt being shown. */
    private final Map<String, CompletableFuture<PermissionPrompter.Answer>> pendingPrompts = new ConcurrentHashMap<>();
    private final AtomicInteger nextReq = new AtomicInteger(1);

    private volatile PermissionPrompter prompter;
    /** Tests: the prompt timeout in ms instead of {@code apps.promptTimeoutSeconds} (-1 = the config). */
    private volatile long promptTimeoutMs = -1;
    private volatile BuiltinLauncher builtinLauncher;
    private volatile AppEvents events = new AppEvents() {};
    private volatile LifecyclePolicy policy = LifecyclePolicy.PC;
    private volatile boolean shutDown;
    private final ScheduledFuture<?> pump;

    /**
     * @param storageRoot     the computer's disk (the guest's {@code /})
     * @param programResolver system program name ({@code "python"}) -> its .wasm, or null
     * @param baseEnv         environment every app gets (terminal size)
     */
    public AppManager(Path storageRoot, ProcessManager pm, Function<String, Path> programResolver,
                      Supplier<Map<String, String>> baseEnv) {
        this.root = storageRoot;
        this.pm = pm;
        this.programResolver = programResolver;
        this.baseEnv = baseEnv;
        this.store = new PermissionStore(storageRoot);
        this.states = new AppStateStore(storageRoot);
        this.background = new BackgroundScheduler(this, pm);
        pm.setSandbox(this);
        pm.addImports((state, sink, pid) -> AppHostFunctions.register(this, state, sink, pid));
        reindex();
        this.pump = IO.scheduleWithFixedDelay(this::pumpSafely, 20, 20, TimeUnit.MILLISECONDS);
    }

    // ================================================================ configuration

    /** The compositor's prompt UI; null = console mode (no-prompter policy). */
    public void setPrompter(@Nullable PermissionPrompter prompter) {
        this.prompter = prompter;
    }

    @Nullable
    public PermissionPrompter prompter() {
        return prompter;
    }

    /** How long a prompt waits for the player: {@code ms}, or the config ({@code apps.promptTimeoutSeconds}) when negative. */
    public void setPromptTimeoutMs(long ms) {
        this.promptTimeoutMs = ms;
    }

    long promptTimeoutMs() {
        long ms = promptTimeoutMs;
        return ms >= 0 ? ms : EcmConfig.appsPromptTimeoutSeconds() * 1000L;
    }

    /** Starts a built-in program (not an app) for a file type: its pid, or -1. */
    @FunctionalInterface
    public interface BuiltinLauncher {
        int launch(String program, List<String> args, int fromPid);
    }

    /** Who starts the built-in handlers ({@link #BUILTIN_HANDLERS}); the computer sets it. */
    public void setBuiltinLauncher(@Nullable BuiltinLauncher launcher) {
        this.builtinLauncher = launcher;
    }

    /** The compositor's message delivery; null = console mode. */
    public void setAppEvents(@Nullable AppEvents events) {
        this.events = events != null ? events : new AppEvents() {};
    }

    AppEvents events() {
        return events;
    }

    /** Terminal = {@link LifecyclePolicy#PC} (default); the ePhone host sets {@link LifecyclePolicy#PHONE}. */
    public void setPolicy(LifecyclePolicy policy) {
        this.policy = policy;
    }

    public LifecyclePolicy policy() {
        return policy;
    }

    /** The computer's disk (the guest's {@code /}). */
    public Path storageRoot() {
        return root;
    }

    public PermissionStore permissions() {
        return store;
    }

    public AppStateStore states() {
        return states;
    }

    /** ePhone background rules: refresh wakes and exception modes. */
    public BackgroundScheduler background() {
        return background;
    }

    // ================================================================ installed apps

    /** Rescan {@code /apps}: each {@code /apps/<id>/app.toml} whose id matches its folder and whose entry exists. */
    public synchronized int reindex() {
        SystemApps.provision(root, programResolver, store);
        Map<String, AppManifest> found = new TreeMap<>();
        Map<String, String> bad = new TreeMap<>();
        Path apps = root.resolve("apps");
        if (Files.isDirectory(apps)) {
            try (var dirs = Files.list(apps)) {
                for (Path d : (Iterable<Path>) dirs::iterator) {
                    if (!Files.isDirectory(d)) continue;
                    String folder = d.getFileName().toString();
                    Path toml = d.resolve("app.toml");
                    if (!Files.isRegularFile(toml)) continue;
                    try {
                        AppManifest m = AppManifest.parse(Files.readString(toml, StandardCharsets.UTF_8));
                        if (!m.id().equals(folder)) throw new AppManifest.ManifestException("id", "'" + m.id() + "' is installed in /apps/" + folder);
                        if (!Files.isRegularFile(d.resolve(m.entry()))) throw new AppManifest.ManifestException("entry", m.entry() + " is missing");
                        found.put(m.id(), m);
                    } catch (AppManifest.ManifestException | IOException e) {
                        bad.put(folder, e.getMessage());
                    }
                }
            } catch (IOException e) {
                EvansComputerMod.LOGGER.warn("[apps] cannot list {}: {}", apps, e.toString());
            }
        }
        installed.keySet().retainAll(found.keySet());
        installed.putAll(found);
        broken.clear();
        broken.putAll(bad);
        bad.forEach((f, why) -> EvansComputerMod.LOGGER.warn("[apps] /apps/{} ignored: {}", f, why));
        return found.size();
    }

    /** Installed apps by id (a copy). */
    public Map<String, AppManifest> installedApps() {
        return new TreeMap<>(installed);
    }

    /** Folders under /apps that aren't valid apps, with the reason. */
    public Map<String, String> brokenApps() {
        return new TreeMap<>(broken);
    }

    @Nullable
    public AppManifest manifest(String appId) {
        return installed.get(appId);
    }

    // ================================================================ identity

    /** The running app {@code pid} belongs to, or null (not an app: unconfined). */
    @Nullable
    public AppInstance instanceOf(int pid) {
        if (pid <= 0) return null;
        int g = pm.getGroup(pid);
        return g > 0 ? instances.get(g) : null;
    }

    /** App id of {@code pid}, or null when it isn't an app. */
    @Nullable
    public String appIdOf(int pid) {
        AppInstance i = instanceOf(pid);
        return i == null ? null : i.appId();
    }

    /** Running apps (a copy). */
    public List<AppInstance> running() {
        List<AppInstance> out = new ArrayList<>();
        for (AppInstance i : instances.values()) if (i.state != AppInstance.State.EXITED) out.add(i);
        out.sort((a, b) -> Integer.compare(a.group(), b.group()));
        return out;
    }

    /**
     * The group a program the kernel starts on behalf of {@code requesterPid}
     * goes into: the requester's app group, or 0 (its own group, unconfined).
     */
    public int groupForSpawn(int requesterPid) {
        AppInstance i = instanceOf(requesterPid);
        if (i == null || i.state == AppInstance.State.EXITED) return 0;
        return isSystemShell(i.appId()) ? 0 : i.group();
    }

    /**
     * Apps that are the computer's own shell (the ePhone's Terminal): what is typed
     * in them runs unconfined, as in a desktop Terminal, while the app itself keeps
     * its identity and lifecycle. Only when the computer installed it itself
     * ({@code .pkg-origin} = {@code builtin}); a package with the same id doesn't count.
     */
    static final java.util.Set<String> SYSTEM_SHELLS = java.util.Set.of("dev.ecm.terminal");

    public boolean isSystemShell(String appId) {
        if (appId == null || !SYSTEM_SHELLS.contains(appId)) return false;
        try {
            Path o = storageRoot().resolve("apps").resolve(appId).resolve(".pkg-origin");
            return Files.isRegularFile(o) && Files.readString(o).strip().equals("builtin");
        } catch (java.io.IOException e) {
            return false;
        }
    }

    // ================================================================ launching

    /** Start an app as a new process group. {@code args} follow the entry in argv. */
    public AppInstance launch(String appId, List<String> args) throws AppException {
        if (shutDown) throw new AppException(AppException.INVALID, "the computer is shutting down");
        AppManifest m = installed.get(appId);
        if (m == null) throw new AppException(AppException.NOT_FOUND, appId + " is not installed");
        String entry = "apps/" + m.id() + "/" + m.entry();
        Path program;
        List<String> argv = new ArrayList<>();
        if (m.isPython()) {
            program = programResolver.apply("python");
            if (program == null || !Files.isRegularFile(program)) throw new AppException(AppException.NOT_FOUND, "python is not available");
            argv.add("python");
            argv.add("/" + entry);
        } else {
            program = root.resolve(entry);
            argv.add("/" + entry);
        }
        argv.addAll(args);
        try {
            Files.createDirectories(root.resolve("var/apps").resolve(m.id()));
        } catch (IOException e) {
            EvansComputerMod.LOGGER.warn("[apps] cannot create /var/apps/{}: {}", m.id(), e.toString());
        }
        Map<String, String> env = new LinkedHashMap<>(baseEnv.get());
        env.put("ECM_APP_ID", m.id());
        env.put("ECM_APP_DATA", "/var/apps/" + m.id());
        // Registered before the process exists, so its first host call is already confined.
        AppInstance inst = new AppInstance(m, NEXT_GROUP.getAndIncrement());
        instances.put(inst.group(), inst);
        int pid = pm.spawn(program, argv.toArray(new String[0]), env, inst.group());
        if (pid < 0) {
            instances.remove(inst.group());
            throw new AppException(AppException.NOT_FOUND, "cannot start " + program.getFileName());
        }
        inst.rootPid = pid;
        EvansComputerMod.LOGGER.info("[apps] launched {} {} as pid {} (group {})", m.id(), m.version(), pid, inst.group());
        return inst;
    }

    /** Stop every process of an app. */
    public void kill(AppInstance inst) {
        background.stopAllModes(inst);
        for (int pid : pm.groupMembers(inst.group())) pm.kill(pid);
    }

    // ================================================================ permissions (ProcessSandbox)

    @Override
    public boolean isConfined(int pid) {
        return instanceOf(pid) != null;
    }

    @Override
    public boolean check(int pid, String permission) {
        AppInstance i = instanceOf(pid);
        return i == null || decide(i, permission, true, pid);
    }

    @Override
    public boolean peek(int pid, String permission) {
        AppInstance i = instanceOf(pid);
        return i == null || decide(i, permission, false, pid);
    }

    /** May this app use {@code permission} (prompting if {@code mayPrompt} and undecided)? */
    public boolean check(AppInstance i, String permission, boolean mayPrompt) {
        return decide(i, permission, mayPrompt, i.pid());
    }

    private boolean decide(AppInstance i, String perm, boolean mayPrompt, int pid) {
        String id = i.appId();
        if (!i.manifest().permissions().contains(perm)) {
            if (mayPrompt) deny(i, perm, "not declared in app.toml");
            return false;
        }
        Boolean d = store.decision(id, perm);
        if (d == null) d = i.sessionAnswers.get(perm);
        if (d != null) {
            if (!d && mayPrompt) deny(i, perm, "denied earlier");
            return d;
        }
        // Background modes are approved by the player in Settings / the phone shell, never mid-run.
        if (!mayPrompt || perm.startsWith("background:")) return false;
        return ask(i, perm, pid);
    }

    private void deny(AppInstance i, String perm, String why) {
        if (!i.denials().contains(perm)) {
            EvansComputerMod.LOGGER.info("[apps] {} denied {}: {}", i.appId(), perm, why);
        }
        i.denied(perm);
    }

    private boolean ask(AppInstance i, String perm, int pid) {
        PermissionPrompter p = prompter;
        String key = i.appId() + "\n" + perm;
        CompletableFuture<PermissionPrompter.Answer> f = null;
        if (p != null) {
            f = pendingPrompts.computeIfAbsent(key, k -> p.prompt(new PermissionPrompter.Request(
                    nextReq.getAndIncrement(), i.appId(), i.manifest().name(), perm, pid)));
        }
        if (f == null) {
            if (EcmConfig.appsAllowWithoutPrompter()) {
                EvansComputerMod.LOGGER.info("[apps] {} allowed {} for this run (no prompter; apps.allowWithoutPrompter)", i.appId(), perm);
                i.sessionAnswers.put(perm, true);
                return true;
            }
            deny(i, perm, "nobody to ask (console mode); grant it with: pkg perm grant " + i.appId() + " " + perm);
            i.sessionAnswers.put(perm, false);
            return false;
        }
        PermissionPrompter.Answer a;
        try {
            a = f.get(promptTimeoutMs(), TimeUnit.MILLISECONDS);
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            // ProcessManager treats "interrupted" as a clean kill (exit 130).
            throw new RuntimeException("permission prompt interrupted");
        } catch (TimeoutException e) {
            EvansComputerMod.LOGGER.info("[apps] {} (pid {}): nobody answered the {} prompt in {} ms", i.appId(), pid, perm, promptTimeoutMs());
            a = PermissionPrompter.Answer.DENY_ONCE;
        } catch (Exception e) {
            a = PermissionPrompter.Answer.DENY_ONCE;
        } finally {
            pendingPrompts.remove(key, f);
        }
        if (a == null) a = PermissionPrompter.Answer.DENY_ONCE;
        if (a.remember()) store.setDecision(i.appId(), perm, a.allow());
        else i.sessionAnswers.put(perm, a.allow());
        if (!a.allow()) deny(i, perm, a.remember() ? "the player said no" : "the player said no (this run)");
        return a.allow();
    }

    /**
     * Public system configuration every app may read (never write), like world-readable files in a
     * Unix /etc: nothing secret, only what a computer tells its programs about the place it is in.
     * {@code /etc/chat.conf}: the Tech Village Discord backend, which Discord connects to as it opens;
     * {@code /etc/browser.conf}: the computer's default home page for the Browser (a house PC's
     * village website).
     */
    public static final Set<String> PUBLIC_CONFIG = Set.of("/etc/chat.conf", "/etc/browser.conf");

    @Override
    public boolean checkPath(int pid, String vpath, boolean write) {
        AppInstance i = instanceOf(pid);
        if (i == null) return true;
        String id = i.appId();
        String own = "/var/apps/" + id;
        if (under(vpath, own)) return true;
        if (under(vpath, "/apps/" + id)) return !write;
        if (under(vpath, PermissionStore.DB_DIR)) return false;
        if (!write && PUBLIC_CONFIG.contains(vpath)) return true;
        Boolean g = store.grantOf(id, vpath);
        if (g != null && (g || !write)) return true;
        List<String> declared = i.manifest().permissions();
        // packages (the App Store): what pkg itself touches - installed apps, repositories, index caches.
        if (declared.contains("packages") && (under(vpath, "/apps") || under(vpath, "/etc/pkg") || under(vpath, "/var/lib/pkg"))) {
            if (vpath.equals("/etc/pkg/signing.key")) return false;
            return decide(i, "packages", true, pid);
        }
        boolean documents = under(vpath, "/home");
        if (documents && declared.contains("files:documents") && decide(i, "files:documents", true, pid)) return true;
        if (declared.contains("files:all")) {
            // The whole disk, except other apps' code and data and the app database.
            if (under(vpath, "/var/apps") || (write && under(vpath, "/apps"))) return false;
            return decide(i, "files:all", true, pid);
        }
        if (documents && !declared.contains("files:documents")) deny(i, "files:documents", "not declared in app.toml");
        return false;
    }

    private static boolean under(String path, String dir) {
        return path.equals(dir) || path.startsWith(dir + "/");
    }

    /** Grant {@code appId} the one file {@code vpath} (file picker answer). */
    public void onFilePicked(int pid, String vpath, boolean write) {
        String id = appIdOf(pid);
        if (id != null && vpath != null && !vpath.isEmpty()) store.grant(id, ProcessSandbox.normalize(vpath), write);
    }

    // ================================================================ open with

    /** Installed apps that list the file's extension in {@code open_with}. */
    public List<String> handlersFor(String vpath) {
        String ext = extension(vpath);
        List<String> out = new ArrayList<>();
        if (ext == null) return out;
        for (AppManifest m : installed.values()) if (m.openWith().contains(ext)) out.add(m.id());
        out.sort(null);
        return out;
    }

    /** The app that opens this file: the default for its type, or the only handler; null = ask (chooser). */
    @Nullable
    public String defaultHandlerFor(String vpath) {
        String ext = extension(vpath);
        if (ext == null) return null;
        List<String> handlers = handlersFor(vpath);
        String d = store.defaultHandler(ext);
        if (d != null && handlers.contains(d)) return d;
        return handlers.size() == 1 ? handlers.get(0) : null;
    }

    public void setDefaultHandler(String ext, @Nullable String appId) {
        store.setDefaultHandler(ext.toLowerCase(java.util.Locale.ROOT), appId);
    }

    /**
     * {@code Open{path}} from Files or an app ({@code fromPid}, 0 = the system):
     * launch the default handler with a one-file read-write grant and pass the
     * path (argv and {@code OpenFile}). An app can only open files it can read.
     *
     * @throws AppException {@link AppException#CHOOSE} when several apps handle it and none is the default
     */
    public AppInstance open(String vpath, int fromPid) throws AppException {
        String path = ProcessSandbox.normalize(vpath);
        if (fromPid > 0 && !checkPath(fromPid, path, false)) throw new AppException(AppException.DENIED, "you can't open " + path);
        String handler = defaultHandlerFor(path);
        if (handler == null) {
            List<String> h = handlersFor(path);
            if (h.isEmpty()) throw new AppException(AppException.NOT_FOUND, "no app opens " + path);
            throw new AppException(AppException.CHOOSE, "choose an app: " + String.join(" ", h));
        }
        return openWith(path, handler);
    }

    /**
     * File types that open with a built-in program (not an app) when no
     * installed app lists them in {@code open_with}: the defaults Files used
     * to hard-code. Text files have an app (the system app {@code dev.ecm.editor}).
     */
    public static final Map<String, String> BUILTIN_HANDLERS = Map.of(".vpl", "visual", ".gba", "gba");

    /** The Text Editor system app: opens text types and files without a type. */
    public static final String TEXT_EDITOR = "dev.ecm.editor";

    /** The built-in program that opens {@code vpath} when no installed app does, or null. */
    @Nullable
    public String builtinHandlerFor(String vpath) {
        String ext = extension(vpath);
        if (ext == null || !handlersFor(vpath).isEmpty()) return null;
        return BUILTIN_HANDLERS.get(ext);
    }

    /** What {@link #openAny} started: an app ({@code app} set) or a built-in program ({@code handler} is its name). */
    public record Opened(String handler, int pid, @Nullable AppInstance app) {}

    /**
     * {@link #open}, falling back to the built-in program for the type
     * ({@link #BUILTIN_HANDLERS}) when no installed app opens it. A built-in
     * started for an app runs in that app's group. Files and {@code Open{path}}
     * use this.
     */
    public Opened openAny(String vpath, int fromPid) throws AppException {
        String path = ProcessSandbox.normalize(vpath);
        String builtin = builtinHandlerFor(path);
        BuiltinLauncher l = builtinLauncher;
        if (builtin != null && l != null) {
            if (fromPid > 0 && !checkPath(fromPid, path, false)) throw new AppException(AppException.DENIED, "you can't open " + path);
            if (!Files.exists(root.resolve(path.substring(1)))) throw new AppException(AppException.NOT_FOUND, path + " does not exist");
            int pid = l.launch(builtin, List.of(path), fromPid);
            if (pid <= 0) throw new AppException(AppException.NOT_FOUND, "cannot start " + builtin);
            EvansComputerMod.LOGGER.info("[apps] opened {} with the built-in {} (pid {})", path, builtin, pid);
            return new Opened(builtin, pid, null);
        }
        if (extension(path) == null && installed.containsKey(TEXT_EDITOR)) {
            // No type (README, Makefile): text, in the Text Editor app.
            if (fromPid > 0 && !checkPath(fromPid, path, false)) throw new AppException(AppException.DENIED, "you can't open " + path);
            AppInstance i = openWith(path, TEXT_EDITOR);
            return new Opened(i.appId(), i.pid(), i);
        }
        AppInstance i = open(path, fromPid);
        return new Opened(i.appId(), i.pid(), i);
    }

    /** Open {@code vpath} with a chosen app (the chooser's answer). */
    public AppInstance openWith(String vpath, String appId) throws AppException {
        String path = ProcessSandbox.normalize(vpath);
        if (!Files.exists(root.resolve(path.substring(1)))) throw new AppException(AppException.NOT_FOUND, path + " does not exist");
        store.grant(appId, path, true);
        AppInstance inst = launch(appId, List.of(path));
        events.openFile(inst, path);
        return inst;
    }

    @Nullable
    private static String extension(String vpath) {
        String name = vpath.substring(vpath.lastIndexOf('/') + 1);
        int dot = name.lastIndexOf('.');
        return dot <= 0 ? null : name.substring(dot).toLowerCase(java.util.Locale.ROOT);
    }

    // ================================================================ lifecycle

    /**
     * Ask apps for their state: {@code WillSuspend{deadlineMs}} to each, then
     * wait until every app that got it has answered with {@code SaveState}
     * (persisted by {@link #onSaveState}) or the deadline passes. Returns the
     * number of states saved.
     */
    public int collectStates(Collection<AppInstance> apps, int deadlineMs) {
        List<CompletableFuture<byte[]>> waits = new ArrayList<>();
        for (AppInstance i : apps) {
            if (i.state == AppInstance.State.EXITED) continue;
            CompletableFuture<byte[]> f = new CompletableFuture<>();
            i.pendingSave = f;
            boolean asked;
            try {
                asked = events.willSuspend(i, deadlineMs);
            } catch (RuntimeException e) {
                EvansComputerMod.LOGGER.warn("[apps] WillSuspend to {} failed: {}", i.appId(), e.toString());
                asked = false;
            }
            if (asked) waits.add(f);
            else i.pendingSave = null;
        }
        long deadline = System.nanoTime() + deadlineMs * 1_000_000L;
        int saved = 0;
        for (CompletableFuture<byte[]> f : waits) {
            long left = Math.max(0, deadline - System.nanoTime());
            try {
                f.get(left, TimeUnit.NANOSECONDS);
                saved++;
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                break;
            } catch (Exception ignored) {
                // missed the deadline: its previous state (if any) stays
            }
        }
        for (AppInstance i : apps) i.pendingSave = null;
        return saved;
    }

    /** An app sent {@code SaveState}: keep it in {@code /var/apps/<id>/state}. */
    public boolean onSaveState(int pid, byte[] data) {
        AppInstance i = instanceOf(pid);
        if (i == null) return false;
        boolean ok = states.save(i.appId(), data);
        CompletableFuture<byte[]> f = i.pendingSave;
        if (f != null) f.complete(data);
        return ok;
    }

    /** True if a {@code Restore} would follow {@code Welcome} for this app (sets {@code restore_follows}). */
    public boolean peekRestore(String appId) {
        return Files.isRegularFile(states.stateFile(appId));
    }

    /** The bytes for {@code Restore}, or null if the app has no saved state. */
    @Nullable
    public byte[] takeRestore(String appId) {
        return states.load(appId);
    }

    /**
     * Shutdown or reboot of a Terminal (PC policy): ask every running app for
     * its state (one deadline for all), remember which apps were running, and
     * stop them. Idempotent; later launches are refused. Returns the number of
     * states saved.
     */
    public int shutdown() {
        if (shutDown) return 0;
        shutDown = true;
        List<AppInstance> live = running();
        List<String> ids = new ArrayList<>();
        for (AppInstance i : live) if (!ids.contains(i.appId())) ids.add(i.appId());
        states.saveSession(ids);
        int saved = live.isEmpty() ? 0 : collectStates(live, EcmConfig.appsSaveStateDeadlineMs());
        if (!live.isEmpty()) {
            EvansComputerMod.LOGGER.info("[apps] shutdown: {} running, {} saved their state", ids, saved);
        }
        for (AppInstance i : live) kill(i);
        return saved;
    }

    /** Apps that were running at the last shutdown (a desktop may relaunch them). */
    public List<String> lastSession() {
        return states.lastSession();
    }

    /**
     * Suspend an app (ePhone: switched away or the phone went to sleep): ask
     * for its state with the save deadline, then pause its whole group.
     */
    public void suspend(AppInstance i) {
        if (i.state != AppInstance.State.RUNNING) return;
        collectStates(List.of(i), EcmConfig.appsSaveStateDeadlineMs());
        i.state = AppInstance.State.SUSPENDED;
        pm.pauseGroup(i.group());
    }

    /** Bring a suspended app back (foreground). */
    public void resume(AppInstance i) {
        if (i.state != AppInstance.State.SUSPENDED) return;
        i.state = AppInstance.State.RUNNING;
        background.endRefresh(i, "resumed");
        pm.resumeGroup(i.group());
    }

    /** The app sent {@code BackgroundDone}: end its refresh wake now. */
    public void onBackgroundDone(int pid) {
        AppInstance i = instanceOf(pid);
        if (i != null) background.onBackgroundDone(i);
    }

    // ================================================================ output pump

    private void pumpSafely() {
        try {
            pump();
        } catch (Throwable t) {
            EvansComputerMod.LOGGER.warn("[apps] output pump failed", t);
        }
    }

    /** Collect root processes' output, notice exits, forget apps whose processes have all ended. */
    void pump() {
        byte[] buf = new byte[4096];
        for (AppInstance i : instances.values()) {
            int pid = i.rootPid;
            if (pid > 0 && i.exitCode == null) {
                int n;
                while ((n = pm.readOutput(pid, buf)) > 0) i.appendOutput(buf, n);
                Integer code = pm.tryWait(pid);
                if (code != null) {
                    while ((n = pm.readOutput(pid, buf)) > 0) i.appendOutput(buf, n);
                    i.exitCode = code == Integer.MIN_VALUE ? -1 : code;
                    pm.readOutput(pid, buf); // reaps the entry
                    EvansComputerMod.LOGGER.info("[apps] {} (pid {}) exited with code {}", i.appId(), pid, i.exitCode);
                }
            }
            if (i.exitCode != null && pm.groupMembers(i.group()).isEmpty()) {
                i.state = AppInstance.State.EXITED;
                background.stopAllModes(i);
                // Kept a minute by root pid so `app_ctl status`/`output` can still report it.
                exited.put(i.pid(), i);
                instances.remove(i.group());
            }
        }
        exited.values().removeIf(i -> exitedTooLong(i));
    }

    /** Exited apps by root pid, kept for {@code app_ctl status}/{@code output} for a minute. */
    private final Map<Integer, AppInstance> exited = new ConcurrentHashMap<>();
    private final Map<Integer, Long> exitedAt = new HashMap<>();

    private synchronized boolean exitedTooLong(AppInstance i) {
        long now = System.currentTimeMillis();
        long t = exitedAt.computeIfAbsent(i.pid(), k -> now);
        if (now - t > 60_000) {
            exitedAt.remove(i.pid());
            return true;
        }
        return false;
    }

    /** A running or recently exited app by its root pid. */
    @Nullable
    public AppInstance byRootPid(int pid) {
        for (AppInstance i : instances.values()) if (i.pid() == pid) return i;
        return exited.get(pid);
    }

    @Override
    public void close() {
        pump.cancel(false);
        background.close();
    }
}
