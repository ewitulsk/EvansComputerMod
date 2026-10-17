package com.example.evanscomputermod.computer;

import org.junit.jupiter.api.Test;

import java.util.*;

import static org.junit.jupiter.api.Assertions.*;

/**
 * The ring's DNS plan (docs/DNS.md): roles spread over the villages from the base village,
 * addresses, and the generated zones and configuration.
 */
class DnsPlanTest {

    @Test
    void rolesAreSpreadOverTheRing() {
        for (long seed : new long[] {1, 42, 123456, -7, 987654321987L}) {
            DnsPlan p = new DnsPlan(seed, false, 3, 5);
            assertTrue(p.base >= 1 && p.base <= 10);
            assertEquals(p.base, DnsPlan.baseVillage(seed), "fixed by the seed");
            Set<Integer> hierarchy = new HashSet<>();
            int perVillageMax = 0;
            for (int v = 1; v <= 10; v++) {
                List<String> roles = p.roles(v);
                assertEquals(DnsPlan.RESOLVER, roles.get(0), "every village has its resolver first");
                perVillageMax = Math.max(perVillageMax, roles.size());
                if (roles.size() > 1) hierarchy.add(v);
            }
            assertEquals(3, perVillageMax, "at most three DNS computers per data center");
            assertEquals(9, hierarchy.size(), "nine villages run a hierarchy role");
            assertEquals(List.of(DnsPlan.RESOLVER), p.roles(p.village(8)), "B+8 only has its resolver");
            // letters on alternate villages, a-e at B+1,3,5,7,9
            Set<Integer> letters = new HashSet<>();
            for (char l : DnsPlan.LETTERS.toCharArray()) {
                int v = p.letterVillage(l);
                assertEquals(l, p.letter(v));
                assertTrue(letters.add(v));
                assertEquals(1, Math.floorMod(v - p.base, 2));
            }
            assertEquals(5, Math.floorMod(p.ecmSecondary() - p.ecmPrimary(), 10), "registry secondaries five villages away");
            assertEquals(5, Math.floorMod(p.ringSecondary() - p.ringPrimary(), 10));
            assertNotEquals(p.iana(), p.registrar());
            int total = 0;
            for (int v = 1; v <= 10; v++) total += p.roles(v).size();
            assertEquals(21, total, "21 new computers");
        }
    }

    @Test
    void resolverOnTheIspRouter() {
        DnsPlan p = new DnsPlan(42, true, 3, 5);
        int total = 0;
        for (int v = 1; v <= 10; v++) {
            assertFalse(p.roles(v).contains(DnsPlan.RESOLVER));
            assertEquals("100." + (64 + v) + ".0.1", p.resolver(v));
            total += p.roles(v).size();
        }
        assertEquals(11, total, "11 new computers");
        Map<String, String> isp = p.files(4, DnsPlan.ISP_ROUTER);
        assertTrue(isp.get("etc/named.conf").contains("recursion yes"));
        assertTrue(isp.containsKey("etc/named/root.hints"));
        assertEquals("named &\n", p.services(4, DnsPlan.ISP_ROUTER));
        assertTrue(new DnsPlan(42, false, 3, 5).files(4, DnsPlan.ISP_ROUTER).isEmpty());
        assertEquals("100.68.0.53", new DnsPlan(42, false, 3, 5).resolver(4));
    }

    @Test
    void zones() {
        DnsPlan p = new DnsPlan(42, false, 3, 5);
        String root = p.rootZone();
        for (char l : DnsPlan.LETTERS.toCharArray()) {
            assertTrue(root.contains("@ NS " + l + ".root-servers.ring."));
            assertTrue(root.contains(l + ".root-servers.ring. A " + p.rootServer(l)));
        }
        assertTrue(root.contains("ring. NS ns1.ring.ring.") && root.contains("ns1.ring.ring. A " + p.ringPrimaryAddress()));
        assertTrue(root.contains("ecm. NS ns1.ecm.ring.") && root.contains("ns2.ecm.ring. A " + p.ecmSecondaryAddress()));
        String ring = p.ringZone();
        for (int v = 1; v <= 10; v++) {
            assertTrue(ring.contains("v" + v + " NS ns.v" + v + "\n"));
            assertTrue(ring.contains("ns.v" + v + " A 100." + (64 + v) + ".0.10\n"));
        }
        assertTrue(ring.contains("nic A " + p.registrarAddress()));
        assertTrue(ring.contains("chat A 100.67.0.20"));
        String v4 = p.villageZone(4);
        assertTrue(v4.contains("$ORIGIN v4.ring."));
        assertTrue(v4.contains("web A 100.68.0.10") && v4.contains("dns A 100.68.0.53") && v4.contains("isp A 100.68.0.1"));
        assertTrue(p.ecmZone().contains("@ NS ns2.ecm.ring."));
        assertTrue(p.rootHints().contains("a.root-servers.ring.    3600000 A  " + p.rootServer('a')));
    }

    @Test
    void configuration() {
        DnsPlan p = new DnsPlan(42, false, 3, 5);
        int iana = p.iana(), ecm = p.ecmPrimary(), ecm2 = p.ecmSecondary(), ring = p.ringPrimary(), reg = p.registrar();
        assertTrue(p.files(iana, DnsPlan.IANA).get("etc/named.conf").contains("zone . primary /etc/named/root.zone"));
        assertTrue(p.services(iana, DnsPlan.IANA).contains("ianad &"));
        char l = p.letter(p.letterVillage('c'));
        assertEquals('c', l);
        assertTrue(p.files(p.letterVillage('c'), DnsPlan.ROOT).get("etc/named.conf").contains("zone . secondary " + p.ianaAddress()));
        String ecmConf = p.files(ecm, DnsPlan.REGISTRY).get("etc/named.conf");
        assertTrue(ecmConf.contains("zone ecm primary /etc/named/ecm.zone notify " + p.ecmSecondaryAddress()));
        assertTrue(p.files(ecm, DnsPlan.REGISTRY).get("etc/registryd.conf").contains("registrar nic-ring " + p.registrarSecret()));
        assertTrue(p.files(ecm2, DnsPlan.REGISTRY).get("etc/named.conf").contains("zone ecm secondary " + p.ecmPrimaryAddress()));
        assertTrue(p.files(ring, DnsPlan.REGISTRY).get("etc/named.conf").contains("zone ring primary"));
        assertTrue(p.services(ecm, DnsPlan.REGISTRY).contains("registryd &"));
        assertFalse(p.services(ecm2, DnsPlan.REGISTRY).contains("registryd"));
        assertTrue(p.files(reg, DnsPlan.REGISTRAR).get("etc/registrard.conf").contains("registry " + p.ecmPrimaryAddress() + " 700"));
        assertEquals("iface eth1 100.68.0.53/24\nroute default via 100.68.0.1 dev eth1\ndns 100.68.0.53\n", p.serverNetwork(4, 53));
        assertEquals(p.registrarSecret(), new DnsPlan(42, true, 1, 1).registrarSecret());
        assertNotEquals(p.registrarSecret(), new DnsPlan(43, false, 3, 5).registrarSecret());
        assertTrue(p.map().get(0).startsWith("Base village B = " + p.base));
    }
}
