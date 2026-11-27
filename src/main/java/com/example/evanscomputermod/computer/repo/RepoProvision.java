package com.example.evanscomputermod.computer.repo;

import com.example.evanscomputermod.EcmConfig;
import com.example.evanscomputermod.EvansComputerMod;
import net.minecraft.server.MinecraftServer;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.UUID;

/**
 * Keeps the official repository in a computer's {@code /etc/pkg/repos} in step
 * with the server's settings ({@code repo.access}, {@code repo.url},
 * {@code repo.key}), each time the computer boots:
 *
 * <ul>
 * <li>{@code open}: {@code official host://repo/r/stable/ <key>} (and
 *     {@code community}), reached through the server, no network needed;
 * <li>{@code internet}: the same channels at {@code https://...}, reached over
 *     the computer's own network;
 * <li>{@code datacenter}: {@code host://} entries on repository servers only;
 *     other computers get none (they add a data center's repository, or find
 *     it on the network);
 * <li>{@code off}, or no key configured: none.
 * </ul>
 *
 * Only the two entries it manages ({@code official} and {@code community},
 * while they point at a channel of the official repository) are touched; every
 * other line, and an entry of that name a player pointed somewhere else, stays.
 */
public final class RepoProvision {
    public static final String HOST_ORIGIN = "host://repo";
    static final String REPOS = "etc/pkg/repos";
    static final String HEADER = "# pkg repositories: <name> <url> [<pinned ed25519 key>]";

    private RepoProvision() {}

    /** The managed lines for a computer: {@code <name> <url> <key>}. */
    public static List<String> wanted(String access, String url, String key, boolean community, boolean repoServer) {
        List<String> out = new ArrayList<>();
        if (key.length() != 64 || !key.chars().allMatch(c -> Character.digit(c, 16) >= 0)) return out;
        String origin = switch (access) {
            case "open" -> HOST_ORIGIN;
            case "internet" -> url;
            case "datacenter" -> repoServer ? HOST_ORIGIN : null;
            default -> null;
        };
        if (origin == null || origin.isEmpty()) return out;
        out.add("official " + origin + "/r/stable/ " + key);
        if (community) out.add("community " + origin + "/r/community/ " + key);
        return out;
    }

    /** A line this class wrote before (whatever the settings were then). */
    static boolean managed(String line) {
        String[] p = line.trim().split("\\s+");
        if (p.length < 2) return false;
        boolean channel = (p[0].equals("official") && p[1].endsWith("/r/stable/")) || (p[0].equals("community") && p[1].endsWith("/r/community/"));
        return channel && (p[1].startsWith("host://repo/") || p[1].startsWith("https://") || p[1].startsWith("http://"));
    }

    /** The new file text, or null when nothing changes. */
    public static String merge(String current, List<String> wanted) {
        List<String> kept = new ArrayList<>();
        boolean header = false;
        for (String line : current.split("\n", -1)) {
            if (line.isBlank()) continue;
            if (line.trim().equals(HEADER)) {
                header = true;
                continue;
            }
            if (!managed(line)) kept.add(line);
        }
        if (wanted.isEmpty() && kept.isEmpty() && !header && current.isBlank()) return null;
        StringBuilder sb = new StringBuilder(HEADER).append('\n');
        // The official entries first: with the same version in two repositories, the first one listed wins.
        for (String l : wanted) sb.append(l).append('\n');
        for (String l : kept) sb.append(l).append('\n');
        String next = sb.toString();
        return next.equals(current) ? null : next;
    }

    /** Bring {@code <root>/etc/pkg/repos} up to date. */
    public static void apply(Path root, MinecraftServer server, UUID computer) {
        try {
            String access = EcmConfig.repoAccess();
            boolean repoServer = access.equals("datacenter") && RepoServers.isServer(server, computer);
            List<String> wanted = wanted(access, EcmConfig.repoUrl(), EcmConfig.repoKey(), EcmConfig.repoAllowCommunity(), repoServer);
            Path file = root.resolve(REPOS);
            String current = Files.isRegularFile(file) ? Files.readString(file, StandardCharsets.UTF_8) : "";
            String next = merge(current, wanted);
            if (next == null) return;
            Files.createDirectories(file.getParent());
            Files.writeString(file, next, StandardCharsets.UTF_8);
        } catch (IOException | RuntimeException e) {
            EvansComputerMod.LOGGER.warn("[repo] cannot update {} for {}: {}", REPOS, computer, e.toString());
        }
    }
}
