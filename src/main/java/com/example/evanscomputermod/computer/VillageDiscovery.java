package com.example.evanscomputermod.computer;

/**
 * Service discovery in the Tech Villages (docs/DISCOVERY.md, docs/TECH_VILLAGE_NETWORK.md
 * "Finding services"): the files {@link WorldNetwork} provisions. The ISP router answers for
 * the ring's chat server and repository; the routers between it and the players (home routers,
 * the carrier core) relay searches to their gateway; the chat and package servers run the
 * responder so they are found on their own LAN too. The lines are what {@code
 * ecm_disco::Config::parse} reads.
 */
public final class VillageDiscovery {
    /** The responder's line in services.cfg. */
    public static final String SERVE = "disco serve &";
    /** Its configuration file. */
    public static final String CONF = "etc/disco.conf";
    /** A router's /etc/disco.conf: searches from its LANs are passed on to its own gateway. */
    public static final String RELAY = "# Service discovery (docs/DISCOVERY.md): pass searches on to the next router\nrelay gateway\n";
    /** What the ring's chat server is called in answers. */
    public static final String CHAT_NAME = "Tech Village chat";

    /**
     * The ISP router's /etc/disco.conf: the chat server at {@code chatAddress}:{@code chatPort}
     * and the package repository ({@code repo}, or null for none).
     */
    public static String ispConf(String chatAddress, int chatPort, VillageRepo repo) {
        String c = "# Service discovery (docs/DISCOVERY.md): the ring's services, answered for the village\n"
                + "record chat " + chatAddress + " " + chatPort + " " + CHAT_NAME + "\n";
        if (repo != null) c += "record pkg " + repo.address() + " " + VillageRepo.PORT + " " + VillageRepo.NAME + "\n";
        return c;
    }

    private VillageDiscovery() {}
}
