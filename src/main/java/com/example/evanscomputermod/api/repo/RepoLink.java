package com.example.evanscomputermod.api.repo;

import java.util.Map;

/**
 * The server's own link to the package repository: HTTPS requests the
 * Minecraft server makes for a computer, so a computer can get packages
 * without any in-game network (docs/REPO.md). The main mod never opens a
 * socket itself; the {@code evanscomputermod-internet} sidecar installs the
 * implementation ({@link RepoLinks#install}).
 *
 * <p>A link only ever talks to the origin it is given, which is the server's
 * configured repository ({@code repo.url}); the path comes from the computer.
 */
public interface RepoLink {
    int CURRENT_API_VERSION = 1;

    int apiVersion();

    String name();

    /**
     * Start a request to {@code origin + path}.
     *
     * @param origin  {@code https://host[:port]}, from the server's configuration
     * @param method  GET, POST, PUT or DELETE
     * @param path    starts with {@code /}; never leaves the origin
     * @param headers already filtered to the ones a computer may send
     * @param body    may be empty
     */
    RepoRequest open(String origin, String method, String path, Map<String, String> headers, byte[] body);
}
