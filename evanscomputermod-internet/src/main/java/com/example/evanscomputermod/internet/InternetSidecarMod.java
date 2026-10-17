package com.example.evanscomputermod.internet;

import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.common.Mod;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * Entry point of the internet sidecar: it links EvansComputerMod's Internet
 * Gateway to the real network. Only the server needs it; the main mod finds
 * {@link SocketUplinkProvider} through ServiceLoader. This sidecar is the only
 * part of EvansComputerMod that opens host sockets or starts a process.
 */
@Mod("evanscomputermod_internet")
public class InternetSidecarMod {
    static final Logger LOGGER = LoggerFactory.getLogger("evanscomputermod_internet");

    public InternetSidecarMod(IEventBus modEventBus) {
        LOGGER.info("Internet sidecar loaded: the Internet Gateway can reach the real internet");
    }
}
