package com.example.evanscomputermod.internet;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.api.repo.RepoRequest;
import com.sun.net.httpserver.HttpServer;
import org.junit.jupiter.api.Test;

import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.atomic.AtomicInteger;

public class HttpsRepoLinkTest {
    private static byte[] drain(RepoRequest r) throws Exception {
        var out = new java.io.ByteArrayOutputStream();
        byte[] buf = new byte[4096];
        long deadline = System.currentTimeMillis() + 10_000;
        while (System.currentTimeMillis() < deadline) {
            int state = r.state();
            int n;
            while ((n = r.read(buf)) > 0) out.write(buf, 0, n);
            if (state == RepoRequest.DONE || state == RepoRequest.FAILED) break;
            Thread.sleep(5);
        }
        return out.toByteArray();
    }

    @Test
    void requestsHeadersCacheAndFailures() throws Exception {
        Path cache = Files.createTempDirectory("ecm-repo-cache");
        AtomicInteger packageHits = new AtomicInteger();
        List<String> seen = new CopyOnWriteArrayList<>();
        byte[] pkg = new byte[70_000];
        for (int i = 0; i < pkg.length; i++) pkg[i] = (byte) (i * 7);
        HttpServer server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        server.createContext("/", ex -> {
            String path = ex.getRequestURI().toString();
            byte[] body = ex.getRequestBody().readAllBytes();
            seen.add(ex.getRequestMethod() + " " + path + " auth=" + ex.getRequestHeaders().getFirst("Authorization") + " body=" + body.length);
            byte[] answer;
            int status = 200;
            if (path.endsWith(".ecmpkg")) {
                packageHits.incrementAndGet();
                answer = pkg;
            } else if (path.equals("/r/stable/index.json")) {
                answer = ("{\"n\":" + seen.size() + "}").getBytes(StandardCharsets.UTF_8);
            } else if (path.equals("/v1/publish")) {
                status = 201;
                answer = "{\"id\":\"dev.t.x\"}".getBytes(StandardCharsets.UTF_8);
            } else if (path.equals("/v1/away")) {
                ex.getResponseHeaders().add("Location", "https://elsewhere.example/");
                ex.sendResponseHeaders(302, -1);
                ex.close();
                return;
            } else {
                status = 404;
                answer = "{\"error\":\"not_found\",\"message\":\"no\"}".getBytes(StandardCharsets.UTF_8);
            }
            ex.sendResponseHeaders(status, answer.length);
            ex.getResponseBody().write(answer);
            ex.close();
        });
        server.start();
        String origin = "http://127.0.0.1:" + server.getAddress().getPort();
        try {
            HttpsRepoLink link = new HttpsRepoLink(cache, () -> 16);
            // A package: fetched once, then served from the server's disk.
            for (int i = 0; i < 3; i++) {
                RepoRequest r = link.open(origin, "GET", "/r/stable/dev.t.x-1.0.0.ecmpkg", Map.of(), new byte[0]);
                assertArrayEquals(pkg, drain(r));
                assertEquals(RepoRequest.DONE, r.state());
                assertEquals(200, r.status());
                assertEquals(pkg.length, r.total());
            }
            assertEquals(1, packageHits.get(), "the second and third came from the cache");
            // The index is always asked for again.
            String a = new String(drain(link.open(origin, "GET", "/r/stable/index.json", Map.of(), new byte[0])));
            String b = new String(drain(link.open(origin, "GET", "/r/stable/index.json", Map.of(), new byte[0])));
            assertNotEquals(a, b);
            // A publish carries the token and the body; an error status is an answer, not a failure.
            RepoRequest put = link.open(origin, "PUT", "/v1/publish", Map.of("Authorization", "Bearer ecm_d_x", "Content-Type", "application/octet-stream"), new byte[1234]);
            assertEquals("{\"id\":\"dev.t.x\"}", new String(drain(put)));
            assertEquals(201, put.status());
            assertTrue(seen.contains("PUT /v1/publish auth=Bearer ecm_d_x body=1234"), seen.toString());
            RepoRequest missing = link.open(origin, "GET", "/v1/packages/dev.t.none", Map.of(), new byte[0]);
            drain(missing);
            assertEquals(RepoRequest.DONE, missing.state());
            assertEquals(404, missing.status());
            // A redirect is handed back, never followed (the token stays with the repository).
            RepoRequest away = link.open(origin, "GET", "/v1/away", Map.of("Authorization", "Bearer ecm_d_x"), new byte[0]);
            drain(away);
            assertEquals(302, away.status());
            // An error page for a package is not cached.
            server.removeContext("/");
            server.createContext("/", ex -> {
                ex.sendResponseHeaders(503, -1);
                ex.close();
            });
            RepoRequest down = link.open(origin, "GET", "/r/stable/dev.t.other-1.0.0.ecmpkg", Map.of(), new byte[0]);
            drain(down);
            assertEquals(503, down.status());
            try (var files = Files.list(cache)) {
                assertEquals(1, files.count(), "only the good package is kept");
            }
            // Origins: https anywhere, plain http only for a local address.
            assertTrue(HttpsRepoLink.originAllowed("https://repo.evanwitulski.com"));
            assertTrue(HttpsRepoLink.originAllowed("http://127.0.0.1:8080"));
            assertFalse(HttpsRepoLink.originAllowed("http://repo.evanwitulski.com"));
            assertFalse(HttpsRepoLink.originAllowed("https://repo.evanwitulski.com/path"));
            assertFalse(HttpsRepoLink.originAllowed("ftp://x"));
            RepoRequest bad = link.open("http://example.com", "GET", "/r/stable/index.json", Map.of(), new byte[0]);
            assertEquals(RepoRequest.FAILED, bad.state());
            assertTrue(bad.error().contains("https://"));
            assertTrue(HttpsRepoLink.cacheable("GET", "/r/stable/a.ecmpkg"));
            assertFalse(HttpsRepoLink.cacheable("GET", "/r/stable/index.json"));
            assertFalse(HttpsRepoLink.cacheable("GET", "/r/stable/a.ecmpkg/icon.png"));
            assertFalse(HttpsRepoLink.cacheable("PUT", "/r/stable/a.ecmpkg"));
        } finally {
            server.stop(0);
        }
        // Nothing listening: a failure with a reason.
        RepoRequest dead = new HttpsRepoLink(cache, () -> 16).open(origin, "GET", "/r/stable/index.json", Map.of(), new byte[0]);
        drain(dead);
        assertEquals(RepoRequest.FAILED, dead.state());
        assertTrue(dead.error().contains("cannot reach the repository"), dead.error());
    }
}
