package com.example.evanscomputermod.computer.repo;

import com.example.evanscomputermod.EcmConfig;
import com.example.evanscomputermod.api.repo.RepoLink;
import com.example.evanscomputermod.api.repo.RepoLinks;
import com.example.evanscomputermod.api.repo.RepoRequest;
import net.minecraft.server.MinecraftServer;

import java.nio.charset.StandardCharsets;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.function.Supplier;

/**
 * A computer's side of the server's link to the package repository: the
 * hardware behind {@code repo_*} (abi/child-abi.toml {@code [imports.repo]}).
 *
 * <p>Who may use it is the server's choice ({@code repo.access}): every
 * computer ({@code open}), only repository servers ({@code datacenter},
 * {@link RepoServers}), or nobody ({@code internet}, {@code off}: computers
 * use their own network, or none). A request can only name a path of the
 * repository's API and package channels, and carries only the headers a
 * package client needs.
 */
public final class RepoPort {
    public static final int SUPPORT_NONE = 0;
    public static final int SUPPORT_READY = 1;
    public static final int SUPPORT_NOT_THIS_COMPUTER = 2;

    public static final int ERR_INVALID = -1;
    public static final int ERR_BUSY = -2;
    public static final int ERR_NO_LINK = -4;
    public static final int ERR_NOT_PERMITTED = -13;

    /** Requests one computer may have open at once. */
    static final int MAX_OPEN = 4;
    /** A package is at most 8 MiB; a little room for the request head. */
    static final int MAX_REQUEST = 9 * 1024 * 1024;

    private static final AtomicInteger NEXT_TICKET = new AtomicInteger(1);

    private final Supplier<UUID> computer;
    private final Supplier<MinecraftServer> server;
    private final Map<Integer, Open> open = new HashMap<>();

    private record Open(int pid, RepoRequest request) {}

    /** {@code repo_poll}'s answer. */
    public record Progress(int state, int status, long received, long total) {}

    public RepoPort(Supplier<UUID> computer, Supplier<MinecraftServer> server) {
        this.computer = computer;
        this.server = server;
    }

    /** One of the {@code SUPPORT_*} values for this computer. */
    public int supported() {
        if (RepoLinks.get() == null) return SUPPORT_NONE;
        return switch (EcmConfig.repoAccess()) {
            case "open" -> SUPPORT_READY;
            case "datacenter" -> RepoServers.isServer(server.get(), computer.get()) ? SUPPORT_READY : SUPPORT_NOT_THIS_COMPUTER;
            default -> SUPPORT_NONE;
        };
    }

    /** A parsed {@code repo_begin} request. */
    record Parsed(String method, String path, Map<String, String> headers, byte[] body) {}

    /** {@code "<METHOD> <path>\n"}, {@code "<Name>: <value>\n"} lines, a blank line, the body. Null if malformed. */
    static Parsed parse(byte[] request) {
        int head = -1;
        for (int i = 0; i + 1 < request.length; i++) {
            if (request[i] == '\n' && request[i + 1] == '\n') {
                head = i;
                break;
            }
        }
        if (head < 0) return null;
        String[] lines = new String(request, 0, head, StandardCharsets.UTF_8).split("\n", -1);
        String[] first = lines[0].split(" ", -1);
        if (first.length != 2) return null;
        String method = first[0], path = first[1];
        if (!java.util.Set.of("GET", "POST", "PUT", "DELETE").contains(method) || !pathAllowed(path)) return null;
        Map<String, String> headers = new LinkedHashMap<>();
        for (int i = 1; i < lines.length; i++) {
            int c = lines[i].indexOf(':');
            if (c <= 0) return null;
            String name = lines[i].substring(0, c).trim(), value = lines[i].substring(c + 1).trim();
            if (value.chars().anyMatch(ch -> ch < 0x20 || ch == 0x7f)) return null;
            String lower = name.toLowerCase(java.util.Locale.ROOT);
            // Only what a package client needs; nothing that could steer the server's HTTP client.
            if (lower.equals("authorization") || lower.equals("content-type") || lower.startsWith("x-ecm-")) headers.put(name, value);
        }
        byte[] body = java.util.Arrays.copyOfRange(request, head + 2, request.length);
        return new Parsed(method, path, headers, body);
    }

    /** Package channels and the repository's API; nothing else on that server. */
    static boolean pathAllowed(String path) {
        if (path.length() > 2048 || path.contains("..") || path.contains("//") || path.contains("\\")) return false;
        if (path.chars().anyMatch(c -> c <= 0x20 || c >= 0x7f || c == '#')) return false;
        return path.startsWith("/r/") || path.startsWith("/v1/") || path.startsWith("/auth/v1/");
    }

    /**
     * Program {@code pid} starts a request. Returns a ticket for {@link #poll},
     * or a negative error.
     */
    public synchronized int begin(int pid, byte[] request) {
        RepoLink link = RepoLinks.get();
        switch (supported()) {
            case SUPPORT_NONE -> { return ERR_NO_LINK; }
            case SUPPORT_NOT_THIS_COMPUTER -> { return ERR_NOT_PERMITTED; }
            default -> { }
        }
        if (request.length > MAX_REQUEST) return ERR_INVALID;
        Parsed p = parse(request);
        if (p == null) return ERR_INVALID;
        if (open.size() >= MAX_OPEN) return ERR_BUSY;
        RepoRequest r;
        if (!EcmConfig.repoAllowCommunity() && p.path.startsWith("/r/community/")) {
            r = new Fixed(404, "{\"error\":\"not_found\",\"message\":\"this server does not offer the community channel\"}");
        } else if (!p.method.equals("GET") && !EcmConfig.repoAllowPublish()) {
            r = new Fixed(403, "{\"error\":\"forbidden\",\"message\":\"this server does not let computers sign in or publish through its link\"}");
        } else {
            r = link.open(EcmConfig.repoUrl(), p.method, p.path, p.headers, p.body);
        }
        int ticket = NEXT_TICKET.getAndIncrement();
        open.put(ticket, new Open(pid, r));
        return ticket;
    }

    private Open mine(int ticket, int pid) {
        Open o = open.get(ticket);
        return o != null && o.pid == pid ? o : null;
    }

    public synchronized Progress poll(int ticket, int pid) {
        Open o = mine(ticket, pid);
        if (o == null) return new Progress(ERR_INVALID, 0, 0, -1);
        RepoRequest r = o.request;
        return new Progress(r.state(), r.status(), r.received(), r.total());
    }

    public int read(int ticket, int pid, byte[] buf) {
        Open o;
        synchronized (this) {
            o = mine(ticket, pid);
        }
        return o == null ? 0 : o.request.read(buf);
    }

    public synchronized String error(int ticket, int pid) {
        Open o = mine(ticket, pid);
        String e = o == null ? null : o.request.error();
        return e == null ? "" : e;
    }

    public synchronized int close(int ticket, int pid) {
        Open o = mine(ticket, pid);
        if (o == null) return 0;
        open.remove(ticket);
        o.request.close();
        return 0;
    }

    /** A program ended: its requests end with it. */
    public synchronized void onProcessExit(int pid) {
        open.entrySet().removeIf(e -> {
            if (e.getValue().pid != pid) return false;
            e.getValue().request.close();
            return true;
        });
    }

    public synchronized void closeAll() {
        open.values().forEach(o -> o.request.close());
        open.clear();
    }

    /** An answer the server gives itself, without asking the repository. */
    private static final class Fixed implements RepoRequest {
        private final int status;
        private final byte[] body;
        private int read;

        Fixed(int status, String body) {
            this.status = status;
            this.body = body.getBytes(StandardCharsets.UTF_8);
        }

        @Override public int state() { return DONE; }
        @Override public int status() { return status; }
        @Override public long received() { return body.length; }
        @Override public long total() { return body.length; }
        @Override public synchronized int read(byte[] buf) {
            int n = Math.min(buf.length, body.length - read);
            System.arraycopy(body, read, buf, 0, n);
            read += n;
            return n;
        }
        @Override public String error() { return null; }
        @Override public void close() {}
    }
}
