package com.example.evanscomputermod.internet;

import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.common.Mod;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * Entry point of the internet sidecar: it links EvansComputerMod's Internet
 * Gateway to the real network. Only the server needs it; the main mod finds
 * {@link SocketUplinkProvider} through ServiceLoader, and this installs the
 * server's link to the package repository ({@link HttpsRepoLink}). This sidecar
 * is the only part of EvansComputerMod that opens host sockets or starts a process.
 */
@Mod("evanscomputermod_internet")
public class InternetSidecarMod {
    static final Logger LOGGER = LoggerFactory.getLogger("evanscomputermod_internet");

    public InternetSidecarMod(IEventBus modEventBus) {
        LOGGER.info("Internet sidecar loaded: the Internet Gateway can reach the real internet");
        // The server's own link to the package repository (repo.access = open or datacenter).
        com.example.evanscomputermod.api.repo.RepoLinks.install(
                new HttpsRepoLink(java.nio.file.Path.of("ecm-repo-cache").toAbsolutePath(), com.example.evanscomputermod.EcmConfig::repoCacheMiB));
    }
}
