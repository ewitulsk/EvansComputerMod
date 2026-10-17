package com.example.evanscomputermod.computer.upload;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.api.upload.UploadBackend;
import com.example.evanscomputermod.api.upload.UploadBackends;
import com.example.evanscomputermod.api.upload.UploadSink;
import com.example.evanscomputermod.api.upload.UploadStatus;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerPlayer;

import java.io.IOException;
import java.io.OutputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.function.Supplier;
import java.util.stream.Stream;

/**
 * A computer's upload port: the hardware behind {@code upload_*}
 * (abi/child-abi.toml {@code [imports.upload]}). A program asks for a file;
 * the player using the computer picks one on their real computer (the
 * {@code evanscomputermod-upload} sidecar's file picker, in their game) and its
 * bytes arrive over the network into the computer's storage.
 *
 * <p>"The player using the computer" is whoever last sent it keyboard or
 * pointer input ({@link #noteOperator}, from {@code ComputerTargets}): the
 * ePhone's holder, or the player at the Terminal. One upload runs at a time
 * per computer. A file is written next to its destination as a hidden
 * {@code .upload-<ticket>.part} and renamed into place when every byte is in,
 * so programs never see half a file.
 */
public final class UploadPort {
    /** {@link #supported()}: no upload add-on on the server. */
    public static final int SUPPORT_NONE = 0;
    /** Ready: the player using the computer can upload. */
    public static final int SUPPORT_READY = 1;
    /** The add-on is on the server, but not in the game of the player using the computer (or nobody is using it). */
    public static final int SUPPORT_NO_CLIENT = 2;
    /** The server doesn't let the player using the computer upload. */
    public static final int SUPPORT_NOT_PERMITTED = 3;

    /** Errors of {@link #begin}. */
    public static final int ERR_INVALID = -1;
    public static final int ERR_BUSY = -2;
    public static final int ERR_NOT_INSTALLED = -4;

    /** File names are cut to this many characters (the extension is kept). */
    static final int MAX_NAME = 64;

    private static final AtomicInteger NEXT_TICKET = new AtomicInteger(1);

    private final Supplier<Path> storage;
    private final Supplier<IComputerHost> host;
    private final Supplier<String> kind;

    private volatile java.lang.ref.WeakReference<ServerPlayer> operator;
    private volatile IComputerHost operatorHost;
    private volatile int operatorSupport = SUPPORT_NO_CLIENT;
    private Session active;

    /**
     * @param storage the computer's own folder (its VFS root)
     * @param host    the computer's host (for the server)
     * @param kind    what to call the computer to the player ("Terminal", "ePhone")
     */
    public UploadPort(Supplier<Path> storage, Supplier<IComputerHost> host, Supplier<String> kind) {
        this.storage = storage;
        this.host = host;
        this.kind = kind;
    }

    /**
     * {@code player} sent input to this computer through {@code via} (the
     * Terminal or ePhone that took it). Server thread.
     */
    public void noteOperator(ServerPlayer player, IComputerHost via) {
        UploadBackend b = UploadBackends.get();
        operator = new java.lang.ref.WeakReference<>(player);
        operatorHost = via;
        operatorSupport = b == null ? SUPPORT_NONE : support(b, player);
    }

    private static int support(UploadBackend b, ServerPlayer player) {
        if (!b.canUpload(player)) return SUPPORT_NO_CLIENT;
        if (!b.permitted(player)) return SUPPORT_NOT_PERMITTED;
        return SUPPORT_READY;
    }

    /** One of the {@code SUPPORT_*} values, for the player who last used this computer. Any thread. */
    public int supported() {
        if (!UploadBackends.available()) return SUPPORT_NONE;
        if (operator == null) return SUPPORT_NO_CLIENT;
        return operatorSupport;
    }

    /**
     * Program {@code pid} asks for a file to be uploaded into directory
     * {@code dest} (a VFS path: {@code /home}, {@code home/pictures}). The
     * player is asked on the server thread. Returns a ticket for
     * {@link #poll}, or {@link #ERR_INVALID} (not a directory),
     * {@link #ERR_BUSY} (another upload is running), {@link #ERR_NOT_INSTALLED}.
     */
    public synchronized int begin(int pid, String dest) {
        UploadBackend backend = UploadBackends.get();
        if (backend == null) return ERR_NOT_INSTALLED;
        String rel = normalizeDir(dest);
        Path root = storage.get();
        if (rel == null || root == null) return ERR_INVALID;
        Path dir = rel.isEmpty() ? root : root.resolve(rel).normalize();
        if (!dir.startsWith(root) || !Files.isDirectory(dir)) return ERR_INVALID;
        if (active != null && !active.closed()) return ERR_BUSY;
        IComputerHost h = host.get();
        MinecraftServer server = h == null ? null : h.getServer();
        Session s = new Session(NEXT_TICKET.getAndIncrement(), pid, root, dir, rel, backend);
        active = s;
        if (server == null) {
            s.fail(UploadStatus.NO_CLIENT);
            return s.ticket;
        }
        var who = operator;
        IComputerHost via = operatorHost;
        server.execute(() -> {
            if (s.closed()) return;
            ServerPlayer player = who == null ? null : who.get();
            if (player == null || player.hasDisconnected() || via == null || !via.canInteract(player, IComputerHost.DISPLAY_OWN)) {
                s.fail(UploadStatus.NO_CLIENT);
                return;
            }
            int support = support(backend, player);
            operatorSupport = support;
            if (support == SUPPORT_NO_CLIENT) {
                s.fail(UploadStatus.NO_CLIENT);
            } else if (support == SUPPORT_NOT_PERMITTED) {
                s.fail(UploadStatus.NOT_PERMITTED);
            } else {
                backend.start(player, s);
            }
        });
        return s.ticket;
    }


    /** Tests: an upload for program {@code pid} into {@code dest} that the backend already started (no server). */
    synchronized Session openForTest(int pid, String dest, UploadBackend backend) {
        String rel = normalizeDir(dest);
        Path root = storage.get();
        Session s = new Session(NEXT_TICKET.getAndIncrement(), pid, root, rel.isEmpty() ? root : root.resolve(rel), rel, backend);
        active = s;
        return s;
    }

    /** What {@link #poll} reports. */
    public record Progress(int state, long received, long total, String path) {}

    /** The state of {@code ticket} (program {@code pid}'s), or {@link UploadStatus#UNKNOWN}. */
    public synchronized Progress poll(int ticket, int pid) {
        Session s = active;
        if (s == null || s.ticket != ticket || s.pid != pid) return new Progress(UploadStatus.UNKNOWN, 0, 0, null);
        return s.progress();
    }

    /** Program {@code pid} stops waiting for {@code ticket}. 0, or {@link UploadStatus#UNKNOWN}. */
    public synchronized int cancel(int ticket, int pid) {
        Session s = active;
        if (s == null || s.ticket != ticket || s.pid != pid) return UploadStatus.UNKNOWN;
        abandon(s);
        return 0;
    }

    /** A program exited: its upload, if any, is cancelled. */
    public synchronized void onProcessExit(int pid) {
        Session s = active;
        if (s != null && s.pid == pid) abandon(s);
    }

    /** The computer stops. */
    public synchronized void close() {
        if (active != null) abandon(active);
        active = null;
    }

    private void abandon(Session s) {
        if (s.closed()) return;
        s.fail(UploadStatus.CANCELLED);
        IComputerHost h = host.get();
        MinecraftServer server = h == null ? null : h.getServer();
        if (server != null) server.execute(() -> s.backend.cancel(s));
    }

    /** {@code dest} as a path relative to the storage root ("" = the root), or null when it isn't a safe path. */
    static String normalizeDir(String dest) {
        if (dest == null) return null;
        String p = dest.trim();
        while (p.startsWith("/")) p = p.substring(1);
        while (p.endsWith("/")) p = p.substring(0, p.length() - 1);
        if (p.length() > 200 || p.indexOf('\\') >= 0 || p.indexOf(':') >= 0 || p.indexOf('\0') >= 0) return null;
        if (p.isEmpty()) return "";
        for (String part : p.split("/")) {
            if (part.isEmpty() || part.equals(".") || part.equals("..")) return null;
        }
        return p;
    }

    /** A file name from the player's computer made safe for the computer's storage. */
    static String sanitizeName(String name) {
        String n = name == null ? "" : name;
        int cut = Math.max(n.lastIndexOf('/'), n.lastIndexOf('\\'));
        if (cut >= 0) n = n.substring(cut + 1);
        StringBuilder b = new StringBuilder();
        for (int i = 0; i < n.length(); i++) {
            char c = n.charAt(i);
            boolean ok = (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9')
                    || c == '.' || c == '_' || c == '-' || c == ' ' || c == '(' || c == ')' || c == '+';
            b.append(ok ? c : '_');
        }
        n = b.toString().trim();
        while (n.startsWith(".")) n = n.substring(1);
        if (n.isEmpty()) n = "upload";
        if (n.length() > MAX_NAME) {
            int dot = n.lastIndexOf('.');
            String ext = dot > 0 && n.length() - dot <= 10 ? n.substring(dot) : "";
            n = n.substring(0, MAX_NAME - ext.length()) + ext;
        }
        return n;
    }

    /** {@code name} in {@code dir}, or "name (1).ext", "name (2).ext", ... when it is taken. */
    static String freeName(Path dir, String name) {
        if (!Files.exists(dir.resolve(name))) return name;
        int dot = name.lastIndexOf('.');
        String base = dot > 0 ? name.substring(0, dot) : name;
        String ext = dot > 0 ? name.substring(dot) : "";
        for (int i = 1; i < 10_000; i++) {
            String n = base + " (" + i + ")" + ext;
            if (!Files.exists(dir.resolve(n))) return n;
        }
        return base + " (" + System.nanoTime() + ")" + ext;
    }

    /** Bytes of every file under {@code root}. */
    static long usedBytes(Path root) {
        try (Stream<Path> walk = Files.walk(root)) {
            return walk.filter(Files::isRegularFile).mapToLong(p -> {
                try {
                    return Files.size(p);
                } catch (IOException e) {
                    return 0;
                }
            }).sum();
        } catch (IOException e) {
            return 0;
        }
    }

    /** One upload. Sink calls come on the server thread, polls on the program's. */
    final class Session implements UploadSink {
        final int ticket;
        final int pid;
        final Path root;
        final Path dir;
        final String rel;
        final UploadBackend backend;
        private int state = UploadStatus.PICKING;
        private long received;
        private long total;
        private String name;
        private Path part;
        private OutputStream out;
        private String path;

        Session(int ticket, int pid, Path root, Path dir, String rel, UploadBackend backend) {
            this.ticket = ticket;
            this.pid = pid;
            this.root = root;
            this.dir = dir;
            this.rel = rel;
            this.backend = backend;
        }

        @Override
        public int ticket() {
            return ticket;
        }

        @Override
        public String destination() {
            return "/" + rel;
        }

        @Override
        public String computerKind() {
            return kind.get();
        }

        @Override
        public synchronized int begin(String fileName, long size) {
            if (state != UploadStatus.PICKING) return closed() ? state : UploadStatus.FAILED;
            if (size < 0) return failWith(UploadStatus.FAILED);
            if (size > backend.maxFileBytes()) return failWith(UploadStatus.TOO_LARGE);
            long limit = backend.maxComputerBytes();
            if (limit > 0 && usedBytes(root) + size > limit) return failWith(UploadStatus.STORAGE_FULL);
            name = sanitizeName(fileName);
            try {
                part = dir.resolve(".upload-" + ticket + ".part");
                out = Files.newOutputStream(part);
            } catch (IOException e) {
                EvansComputerMod.LOGGER.warn("[upload] can't write into {}: {}", dir, e.toString());
                return failWith(UploadStatus.FAILED);
            }
            total = size;
            state = UploadStatus.RECEIVING;
            return UploadStatus.OK;
        }

        @Override
        public synchronized int write(byte[] data, int offset, int length) {
            if (state != UploadStatus.RECEIVING) return closed() ? state : UploadStatus.FAILED;
            if (length < 0 || received + length > total) return failWith(UploadStatus.FAILED);
            try {
                out.write(data, offset, length);
            } catch (IOException e) {
                EvansComputerMod.LOGGER.warn("[upload] write failed: {}", e.toString());
                return failWith(UploadStatus.FAILED);
            }
            received += length;
            return UploadStatus.OK;
        }

        @Override
        public synchronized int finish() {
            if (state != UploadStatus.RECEIVING) return closed() ? state : UploadStatus.FAILED;
            if (received != total) return failWith(UploadStatus.FAILED);
            try {
                out.close();
                out = null;
                String finalName = freeName(dir, name);
                Files.move(part, dir.resolve(finalName), StandardCopyOption.ATOMIC_MOVE);
                path = (rel.isEmpty() ? "/" : "/" + rel + "/") + finalName;
            } catch (IOException e) {
                EvansComputerMod.LOGGER.warn("[upload] can't place the file in {}: {}", dir, e.toString());
                return failWith(UploadStatus.FAILED);
            }
            state = UploadStatus.DONE;
            EvansComputerMod.LOGGER.info("[upload] {} bytes uploaded to {}", total, path);
            return UploadStatus.OK;
        }

        @Override
        public synchronized void fail(int status) {
            if (closed()) return;
            state = status < 0 ? status : UploadStatus.FAILED;
            discard();
        }

        private int failWith(int status) {
            fail(status);
            return state;
        }

        private void discard() {
            try {
                if (out != null) out.close();
            } catch (IOException ignored) {
            }
            out = null;
            try {
                if (part != null) Files.deleteIfExists(part);
            } catch (IOException ignored) {
            }
        }

        @Override
        public synchronized String path() {
            return path;
        }

        @Override
        public synchronized boolean closed() {
            return state == UploadStatus.DONE || state < 0;
        }

        synchronized Progress progress() {
            return new Progress(state, received, total, path);
        }
    }
}
