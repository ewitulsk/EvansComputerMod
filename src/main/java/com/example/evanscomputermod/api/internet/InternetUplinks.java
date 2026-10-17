package com.example.evanscomputermod.api.internet;

import com.example.evanscomputermod.EvansComputerMod;

import java.util.ServiceLoader;

/**
 * The installed {@link InternetUplinkProvider}, if any: discovered once with
 * {@link ServiceLoader} (the {@code evanscomputermod-internet} sidecar
 * registers one); the first one with the current API version wins.
 */
public final class InternetUplinks {
    /** The Internet Gateway's MAC address on every segment that reaches it (it answers ARP for 10.0.0.1). */
    public static final byte[] GATEWAY_MAC = {2, 0, 0, 0, 0, 1};

    private static volatile boolean looked;
    private static volatile InternetUplinkProvider selected;

    private InternetUplinks() {}

    /** The provider, or null when no internet sidecar is installed. */
    public static InternetUplinkProvider get() {
        if (!looked) select();
        return selected;
    }

    /** Whether the Internet Gateway can reach the real network on this server (a provider is installed). */
    public static boolean installed() {
        return get() != null;
    }

    private static synchronized void select() {
        if (looked) return;
        InternetUplinkProvider found = null;
        try {
            for (InternetUplinkProvider p :
                    ServiceLoader.load(InternetUplinkProvider.class, InternetUplinks.class.getClassLoader())) {
                if (p.apiVersion() != InternetUplinkProvider.CURRENT_API_VERSION) {
                    EvansComputerMod.LOGGER.warn("Internet uplink '{}' rejected: apiVersion {} != {}", p.name(),
                            p.apiVersion(), InternetUplinkProvider.CURRENT_API_VERSION);
                    continue;
                }
                found = p;
                break;
            }
        } catch (Throwable t) {
            EvansComputerMod.LOGGER.warn("Looking for an internet uplink failed: {}", t.toString());
        }
        selected = found;
        looked = true;
        if (found == null) {
            EvansComputerMod.LOGGER.info(
                    "No internet uplink installed (evanscomputermod-internet): the Internet Gateway is offline");
        } else {
            EvansComputerMod.LOGGER.info("Internet uplink: {}", found.name());
        }
    }

    /** Tests: use this provider (null = none), or look again with {@link #reset()}. */
    public static synchronized void setForTesting(InternetUplinkProvider provider) {
        selected = provider;
        looked = true;
    }

    public static synchronized void reset() {
        selected = null;
        looked = false;
    }
}
