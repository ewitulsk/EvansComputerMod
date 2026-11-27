package com.example.evanscomputermod.api.repo;

import com.example.evanscomputermod.EvansComputerMod;

/** The installed {@link RepoLink}, if any (the internet sidecar installs one). */
public final class RepoLinks {
    private static volatile RepoLink installed;

    private RepoLinks() {}

    /** Called by the sidecar. A link for another API version is refused. */
    public static synchronized void install(RepoLink link) {
        if (link.apiVersion() != RepoLink.CURRENT_API_VERSION) {
            EvansComputerMod.LOGGER.warn("Repository link '{}' rejected: apiVersion {} != {}", link.name(), link.apiVersion(),
                    RepoLink.CURRENT_API_VERSION);
            return;
        }
        installed = link;
        EvansComputerMod.LOGGER.info("Repository link: {}", link.name());
    }

    /** The link, or null when the server cannot reach the repository itself. */
    public static RepoLink get() {
        return installed;
    }

    /** Tests: use this link (null = none). */
    public static synchronized void setForTesting(RepoLink link) {
        installed = link;
    }
}
