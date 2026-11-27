package com.example.evanscomputermod.internet;

import com.example.evanscomputermod.api.repo.RepoLink;
import com.example.evanscomputermod.api.repo.RepoRequest;

import java.io.IOException;
import java.io.InputStream;
import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.security.MessageDigest;
import java.time.Duration;
import java.util.ArrayDeque;
import java.util.Comparator;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.SynchronousQueue;
import java.util.concurrent.ThreadPoolExecutor;
import java.util.concurrent.TimeUnit;
import java.util.function.IntSupplier;

/**
 * The server's link to the package repository: plain HTTPS requests made by
 * the Minecraft server for a computer (see {@link RepoLink}).
 *
 * <p>Packages are kept on disk once fetched ({@code ecm-repo-cache/} in the
 * server's folder): a published version never changes, so the second computer
 * to install it costs the repository nothing. Indexes and API calls are never
 * cached. The cache is trimmed, oldest first, to {@code repo.cacheMiB}.
 *
 * <p>No redirects are followed and only {@code https://} origins are used
 * (plain {@code http://} for a loopback or LAN address, which is how the
 * tests and a local backend run): the token a computer sends never goes
 * anywhere but the configured repository.
 */
public final class HttpsRepoLink implements RepoLink {
    /** The largest answer handed to a computer. */
    static final long MAX_BODY = 16L * 1024 * 1024;

    private final HttpClient client = HttpClient.newBuilder()
            .connectTimeout(Duration.ofSeconds(15))
            .followRedirects(HttpClient.Redirect.NEVER)
            .version(HttpClient.Version.HTTP_1_1)
            .build();
    private final ExecutorService pool = new ThreadPoolExecutor(0, 16, 30, TimeUnit.SECONDS, new SynchronousQueue<>(), r -> {
        Thread t = new Thread(r, "ECM-Repo-Link");
        t.setDaemon(true);
        return t;
    }, new ThreadPoolExecutor.AbortPolicy());
    private final Path cacheDir;
    private final IntSupplier cacheMiB;

    public HttpsRepoLink(Path cacheDir, IntSupplier cacheMiB) {
        this.cacheDir = cacheDir;
        this.cacheMiB = cacheMiB;
    }

    @Override
    public int apiVersion() {
        return CURRENT_API_VERSION;
    }

    @Override
    public String name() {
        return "evanscomputermod-internet (HTTPS, packages cached in " + cacheDir + ")";
    }

    /** Is this an origin the link may talk to? */
    static boolean originAllowed(String origin) {
        try {
            URI u = URI.create(origin);
            if (u.getHost() == null || u.getPath() != null && !u.getPath().isEmpty() || u.getQuery() != null || u.getUserInfo() != null) return false;
            if ("https".equals(u.getScheme())) return true;
            // A local backend (tests, a LAN deployment) has no certificate.
            if (!"http".equals(u.getScheme())) return false;
            var a = java.net.InetAddress.getByName(u.getHost());
            return a.isLoopbackAddress() || a.isSiteLocalAddress();
        } catch (Exception e) {
            return false;
        }
    }

    /** Package files are immutable once published: only those are cached. */
    static boolean cacheable(String method, String path) {
        return method.equals("GET") && path.startsWith("/r/") && path.endsWith(".ecmpkg") && !path.contains("?");
    }

    private Path cacheFile(String origin, String path) {
        try {
            byte[] h = MessageDigest.getInstance("SHA-256").digest((origin + path).getBytes(StandardCharsets.UTF_8));
            StringBuilder sb = new StringBuilder();
            for (int i = 0; i < 16; i++) sb.append(String.format("%02x", h[i]));
            return cacheDir.resolve(sb + ".ecmpkg");
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }

    @Override
    public RepoRequest open(String origin, String method, String path, Map<String, String> headers, byte[] body) {
        Transfer t = new Transfer();
        if (!originAllowed(origin)) {
            t.fail("the server's repository address (repo.url) must be https://");
            return t;
        }
        Path cached = cacheable(method, path) ? cacheFile(origin, path) : null;
        try {
            pool.execute(() -> run(t, origin, method, path, headers, body, cached));
        } catch (java.util.concurrent.RejectedExecutionException e) {
            t.fail("the server is busy with other repository requests; try again");
        }
        return t;
    }

    private void run(Transfer t, String origin, String method, String path, Map<String, String> headers, byte[] body, Path cached) {
        try {
            if (cached != null && Files.isRegularFile(cached)) {
                byte[] data = Files.readAllBytes(cached);
                try {
                    Files.setLastModifiedTime(cached, java.nio.file.attribute.FileTime.fromMillis(System.currentTimeMillis()));
                } catch (IOException ignored) {
                }
                t.answered(200, data.length);
                t.append(data, data.length);
                t.done();
                return;
            }
            HttpRequest.Builder b = HttpRequest.newBuilder(URI.create(origin + path))
                    .timeout(Duration.ofSeconds(120))
                    .header("User-Agent", "evanscomputermod-server/1")
                    .method(method, body.length == 0 && method.equals("GET") ? HttpRequest.BodyPublishers.noBody() : HttpRequest.BodyPublishers.ofByteArray(body));
            headers.forEach(b::header);
            HttpResponse<InputStream> r = client.send(b.build(), HttpResponse.BodyHandlers.ofInputStream());
            long length = r.headers().firstValueAsLong("Content-Length").orElse(-1);
            if (length > MAX_BODY) {
                r.body().close();
                t.fail("the answer is larger than " + MAX_BODY + " bytes");
                return;
            }
            t.answered(r.statusCode(), length);
            java.io.ByteArrayOutputStream keep = cached != null && r.statusCode() == 200 ? new java.io.ByteArrayOutputStream() : null;
            try (InputStream in = r.body()) {
                byte[] buf = new byte[32 * 1024];
                int n;
                while ((n = in.read(buf)) > 0) {
                    if (t.closed) return;
                    if (t.received() + n > MAX_BODY) {
                        t.fail("the answer is larger than " + MAX_BODY + " bytes");
                        return;
                    }
                    t.append(buf, n);
                    if (keep != null) keep.write(buf, 0, n);
                }
            }
            if (keep != null && (length < 0 || keep.size() == length)) store(cached, keep.toByteArray());
            t.done();
        } catch (java.net.http.HttpTimeoutException e) {
            t.fail("the repository did not answer in time");
        } catch (java.net.ConnectException | java.nio.channels.UnresolvedAddressException e) {
            t.fail("the server cannot reach the repository (is it online?)");
        } catch (javax.net.ssl.SSLException e) {
            t.fail("the repository's certificate was refused: " + e.getMessage());
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            t.fail("interrupted");
        } catch (Exception e) {
            String m = e.getMessage();
            t.fail("the request to the repository failed" + (m == null || m.isBlank() ? "" : ": " + m));
        }
    }

    private synchronized void store(Path file, byte[] data) {
        try {
            Files.createDirectories(cacheDir);
            Path tmp = file.resolveSibling(file.getFileName() + ".tmp");
            Files.write(tmp, data);
            Files.move(tmp, file, StandardCopyOption.REPLACE_EXISTING);
            trim();
        } catch (IOException e) {
            InternetSidecarMod.LOGGER.warn("Cannot keep a package in {}: {}", cacheDir, e.toString());
        }
    }

    /** Delete the least recently used packages until the cache fits. */
    private void trim() throws IOException {
        long cap = Math.max(16, cacheMiB.getAsInt()) * 1024L * 1024L;
        List<Path> files;
        try (var s = Files.list(cacheDir)) {
            files = s.filter(p -> p.getFileName().toString().endsWith(".ecmpkg")).sorted(Comparator.comparingLong(p -> p.toFile().lastModified())).toList();
        }
        long total = 0;
        for (Path p : files) total += Files.size(p);
        for (Path p : files) {
            if (total <= cap) break;
            total -= Files.size(p);
            Files.deleteIfExists(p);
        }
    }

    /** One request's answer, filled by a worker and drained by the computer. */
    private static final class Transfer implements RepoRequest {
        private final ArrayDeque<byte[]> chunks = new ArrayDeque<>();
        private int offset;
        private volatile int state = CONNECTING, status;
        private volatile long received, total = -1;
        private volatile String error;
        volatile boolean closed;

        synchronized void answered(int status, long total) {
            this.status = status;
            this.total = total;
            if (state == CONNECTING) state = ANSWERED;
        }

        synchronized void append(byte[] buf, int n) {
            chunks.addLast(java.util.Arrays.copyOf(buf, n));
            received += n;
        }

        synchronized void done() {
            if (state != FAILED) state = DONE;
        }

        synchronized void fail(String why) {
            error = why;
            state = FAILED;
            chunks.clear();
        }

        @Override public int state() { return state; }
        @Override public int status() { return status; }
        @Override public long received() { return received; }
        @Override public long total() { return total; }
        @Override public String error() { return error; }

        @Override
        public synchronized int read(byte[] buf) {
            int copied = 0;
            while (copied < buf.length && !chunks.isEmpty()) {
                byte[] head = chunks.peekFirst();
                int n = Math.min(buf.length - copied, head.length - offset);
                System.arraycopy(head, offset, buf, copied, n);
                copied += n;
                offset += n;
                if (offset == head.length) {
                    chunks.pollFirst();
                    offset = 0;
                }
            }
            return copied;
        }

        @Override
        public synchronized void close() {
            closed = true;
            chunks.clear();
        }
    }
}
