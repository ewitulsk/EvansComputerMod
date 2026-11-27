package com.example.evanscomputermod.internet;

import com.example.evanscomputermod.EcmConfig;

import java.net.InetAddress;
import java.net.URI;
import java.util.HashSet;
import java.util.Set;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;

/**
 * With {@code repo.access = datacenter} (or {@code off}) computers are not
 * meant to reach the official package repository over the Internet Gateway:
 * packages come from the server's repository servers, and only what those
 * offer. This keeps the proxy's block list at the repository's addresses,
 * looked up again every few minutes.
 *
 * <p>It blocks addresses, not names: a computer that resolves the name still
 * gets a refused connection. Other internet sites are untouched.
 */
final class RepoBlock {
    private static ScheduledExecutorService timer;

    private RepoBlock() {}

    /** The addresses to block for these settings (empty unless the repository is kept from computers). */
    static Set<Integer> resolve(String access, String url) {
        Set<Integer> out = new HashSet<>();
        if (!access.equals("datacenter") && !access.equals("off")) return out;
        try {
            String host = URI.create(url).getHost();
            if (host == null) return out;
            for (InetAddress a : InetAddress.getAllByName(host)) {
                byte[] b = a.getAddress();
                if (b.length == 4) out.add(InternetProxy.i32(b, 0));
            }
        } catch (Exception e) {
            // No DNS right now: keep the list as it was.
            return null;
        }
        return out;
    }

    static void refresh() {
        try {
            Set<Integer> next = resolve(EcmConfig.repoAccess(), EcmConfig.repoUrl());
            if (next != null && !next.equals(InternetProxy.BLOCKED)) {
                InternetProxy.BLOCKED = Set.copyOf(next);
                if (!next.isEmpty()) {
                    InternetSidecarMod.LOGGER.info("The package repository's {} address(es) are closed to computers on the Internet Gateway (repo.access = {})",
                            next.size(), EcmConfig.repoAccess());
                }
            }
        } catch (RuntimeException e) {
            InternetSidecarMod.LOGGER.warn("Cannot update the repository block list: {}", e.toString());
        }
    }

    /** Start refreshing (once). */
    static synchronized void start() {
        if (timer != null) return;
        timer = Executors.newSingleThreadScheduledExecutor(r -> {
            Thread t = new Thread(r, "ECM-Repo-Block");
            t.setDaemon(true);
            return t;
        });
        timer.scheduleWithFixedDelay(RepoBlock::refresh, 0, 300, TimeUnit.SECONDS);
    }
}
