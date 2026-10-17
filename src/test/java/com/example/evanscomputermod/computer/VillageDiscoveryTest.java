package com.example.evanscomputermod.computer;

import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.*;

/**
 * The Tech Village discovery files ({@link VillageDiscovery}, docs/DISCOVERY.md): the ISP's
 * records name the ring's chat server and the world's repository in the shape {@code
 * ecm_disco::Config::parse} reads ({@code record <type> <host> <port> <name>}), routers relay.
 * Where WorldNetwork puts them (1.21.1) is checked in game: {@code tech_village_repo} and the
 * Tech Village client checks.
 */
class VillageDiscoveryTest {
    @Test
    void theIspAnswersForTheRingsServices() {
        VillageRepo repo = VillageRepo.of(73198425L, 4);
        String conf = VillageDiscovery.ispConf("100.68.0.20", 7777, repo);
        assertTrue(conf.contains("\nrecord discord 100.68.0.20 7777 Tech Village\n"), conf);
        assertTrue(conf.contains("\nrecord chat 100.68.0.20 7777 Tech Village\n"), conf);
        assertTrue(conf.contains("\nrecord pkg 100.68.0.21 8080 Tech-Village-Apps\n"), conf);
        assertFalse(conf.contains("relay"), "the ISP is the end of the line: " + conf);
        for (String l : conf.split("\n"))
            assertTrue(l.startsWith("#") || l.matches("record [a-z0-9-]{1,16} \\d+\\.\\d+\\.\\d+\\.\\d+ \\d{1,5} [ -~]{1,48}"), l);
        // Control: a ring without a repository names only the chat server.
        assertFalse(VillageDiscovery.ispConf("100.68.0.20", 7777, null).contains("record pkg"));
    }

    @Test
    void routersRelayToTheirGateway() {
        assertTrue(VillageDiscovery.RELAY.lines().anyMatch("relay gateway"::equals), VillageDiscovery.RELAY);
        assertEquals("disco serve &", VillageDiscovery.SERVE);
        assertEquals("etc/disco.conf", VillageDiscovery.CONF);
    }
}
