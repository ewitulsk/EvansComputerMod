package com.example.evanscomputermod.api.internet;

import java.io.IOException;
import java.util.function.Consumer;

/**
 * Service-provider interface for the Internet Gateway's link to the real
 * network. The main mod has none of its own (it opens no host sockets and
 * starts no processes): the optional {@code evanscomputermod-internet}
 * sidecar registers one, found with {@link java.util.ServiceLoader} (see
 * {@link InternetUplinks}). Without a provider the Internet Gateway is
 * offline and in-game networks only reach each other.
 */
public interface InternetUplinkProvider {
    /** Bumped when this interface changes incompatibly; a provider with another version is ignored. */
    int CURRENT_API_VERSION = 1;

    int apiVersion();

    /** Shown in logs. */
    String name();

    /**
     * Open the uplink for this server. {@code injector} delivers frames from
     * the real network to the Internet Gateway's segments; it is safe to call
     * from any thread. Null when the provider is installed but turned off by
     * the server's settings.
     */
    InternetUplink open(Consumer<byte[]> injector) throws Exception;

    /**
     * Scenarios: serve one plain-HTTP page on this machine, standing in for a
     * website on the internet, that the uplink lets computers reach. Null when
     * the provider can't do that.
     */
    default TestSite openTestSite(String body) throws IOException {
        return null;
    }

    /** A page served by {@link #openTestSite}. */
    interface TestSite extends AutoCloseable {
        /** {@code http://<address>:<port>/}, an address computers can reach through the gateway. */
        String url();

        @Override
        void close();
    }
}
