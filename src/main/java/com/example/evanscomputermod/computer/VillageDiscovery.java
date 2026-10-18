package com.example.evanscomputermod.computer;

/**
 * Service discovery in the Tech Villages (docs/DISCOVERY.md, docs/TECH_VILLAGE_NETWORK.md
 * "Finding services"): the files {@link WorldNetwork} provisions. The ISP router answers for
 * the ring's Discord backend and repository; the routers between it and the players (home routers,
 * the carrier core) relay searches to their gateway; the Discord and package servers run the
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
    /** What the ring's Discord backend (and its home server) is called. */
    public static final String DISCORD_NAME = "Tech Village";

    /**
     * The ISP router's /etc/disco.conf: the Discord backend at {@code chatAddress}:{@code chatPort}
     * (as {@code discord} for the Discord app and {@code chat} for the chat command) and the
     * package repository ({@code repo}, or null for none).
     */
    public static String ispConf(String chatAddress, int chatPort, VillageRepo repo) {
        return ispConf(chatAddress, chatPort, repo, false);
    }

    /**
     * {@link #ispConf(String, int, VillageRepo)}, plus (with {@code accounts}) the ring's realm
     * ({@code auth}, authd) and music server ({@code music}, musicd) on the chat server. Apps use
     * these only to find an address: trust comes from the pinned realm key (docs/AUTH.md).
     */
    public static String ispConf(String chatAddress, int chatPort, VillageRepo repo, boolean accounts) {
        String c = "# Service discovery (docs/DISCOVERY.md): the ring's services, answered for the village\n"
                + "record discord " + chatAddress + " " + chatPort + " " + DISCORD_NAME + "\n"
                + "record chat " + chatAddress + " " + chatPort + " " + DISCORD_NAME + "\n";
        if (accounts) {
            c += "record auth " + chatAddress + " " + VillageAuth.AUTH_PORT + " " + VillageAuth.NAME + "\n"
                    + "record music " + chatAddress + " " + VillageAuth.MUSIC_PORT + " " + VillageAuth.MUSIC_NAME + "\n";
        }
        if (repo != null) c += "record pkg " + repo.address() + " " + VillageRepo.PORT + " " + VillageRepo.NAME + "\n";
        return c;
    }

    private VillageDiscovery() {}
}
