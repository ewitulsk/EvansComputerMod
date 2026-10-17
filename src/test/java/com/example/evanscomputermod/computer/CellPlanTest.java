package com.example.evanscomputermod.computer;

import org.junit.jupiter.api.Test;

import java.util.*;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.*;

/**
 * The Tech Village carrier plan (Cellular v2 phase 5): span-site placement and spacing along
 * the generated fiber chords, and deterministic SIM / site provisioning.
 */
class CellPlanTest {
    /** A two-panel ring like WorldNetwork.plan builds: ten sites around a centre, steered by up to 1500 blocks. */
    static FiberChords ring(long seed) {
        Random r = new Random(seed);
        int n = 10;
        double offset = r.nextDouble() * Math.PI * 2;
        int[][] site = new int[n][];
        for (int i = 0; i < n; i++) {
            double a = offset + i * Math.PI / 5;
            int x = (int) Math.round(5000 * Math.cos(a)) + r.nextInt(3001) - 1500;
            int z = (int) Math.round(5000 * Math.sin(a)) + r.nextInt(3001) - 1500;
            site[i] = new int[] {x >> 4 << 4, 62 + r.nextInt(60) + 24, z >> 4 << 4};
        }
        List<int[]> next = new ArrayList<>(), prev = new ArrayList<>();
        for (int i = 0; i < n; i++) {
            int[] p = site[(i + n - 1) % n], q = site[(i + 1) % n];
            int[] sides = FiberChords.sides(site[i][0], site[i][2], p[0], p[2], q[0], q[2]);
            int[] sp = FiberChords.step(sides[0]), sn = FiberChords.step(sides[1]);
            prev.add(new int[] {site[i][0] + sp[0], site[i][1] + 1, site[i][2] + sp[1]});
            next.add(new int[] {site[i][0] + sn[0], site[i][1] + 1, site[i][2] + sn[1]});
        }
        return new FiberChords(next, prev);
    }

    static double horizontal(int[] a, int[] b) {
        return Math.hypot(a[0] - b[0], a[2] - b[2]);
    }

    @Test
    void spanSitesKeepEveryGapWithinTheTickingSpacing() {
        for (long seed : new long[] {73198425L, 123456L, -42L, 7L, 999_999_937L}) {
            FiberChords ring = ring(seed);
            CellPlan plan = CellPlan.of(seed, ring);
            for (int c = 0; c < 10; c++) {
                int[][] path = ring.path(c);
                List<CellPlan.SpanSite> sites = plan.spans(c);
                double total = horizontal(path[0], path[path.length - 1]);
                assertEquals(Math.max(0, (int) Math.ceil(total / CellPlan.NOMINAL) - 1), sites.size(),
                        "seed " + seed + " chord " + c + ": every planned site placed");
                assertTrue(sites.size() <= CellPlan.MAX_SPANS);
                int[] last = path[0];
                int lastIndex = 0;
                for (CellPlan.SpanSite s : sites) {
                    int[] at = path[s.index()];
                    double gap = horizontal(last, at);
                    assertTrue(gap <= CellPlan.SPACING, "seed " + seed + " chord " + c + " site " + s.k() + " gap " + gap);
                    assertTrue(gap > 100, "sites are spread out, gap " + gap);
                    assertTrue(s.index() > lastIndex, "sites in path order");
                    last = at;
                    lastIndex = s.index();
                }
                double tail = horizontal(last, path[path.length - 1]);
                assertTrue(tail <= CellPlan.SPACING, "seed " + seed + " chord " + c + " last gap " + tail);
            }
        }
    }

    @Test
    void aSiteTapsExactlyItsOwnSpanAndStandsBesideThePath() {
        long seed = 73198425L;
        FiberChords ring = ring(seed);
        CellPlan plan = CellPlan.of(seed, ring);
        Set<Long> pathColumns = new HashSet<>();
        for (int c = 0; c < 10; c++) for (int[] p : ring.path(c)) pathColumns.add(FiberChords.pack(p[0], 0, p[2]));
        Set<String> addresses = new HashSet<>();
        Set<Integer> cells = new HashSet<>();
        assertFalse(plan.allSpans().isEmpty());
        for (CellPlan.SpanSite s : plan.allSpans()) {
            int[] p = ring.path(s.chord())[s.index()];
            long panel = FiberChords.pack(s.x(), s.panelY(), s.z());
            assertEquals(0, ring.chordsAt(panel), "the panel is not itself a path block");
            int touching = 0;
            for (int d = 0; d < 6; d++) {
                long n = SegmentGraph.neighbour(panel, d);
                if (ring.chordsAt(n) != 0) {
                    touching++;
                    assertEquals(FiberChords.pack(p[0], p[1], p[2]), n, "the panel touches its own span");
                }
            }
            assertEquals(1, touching, "the panel touches exactly one span");
            assertTrue(RingPieces.touchesPath(ring, panel), "a tap by the ring's own rule");
            int cx = s.x() >> 4, cz = s.z() >> 4;
            for (int[] col : s.columns()) {
                assertFalse(pathColumns.contains(FiberChords.pack(col[0], 0, col[1])), "a site column is on the path: " + s);
                assertEquals(cx, col[0] >> 4, "one chunk");
                assertEquals(cz, col[1] >> 4, "one chunk");
            }
            assertNotEquals(s.out(), s.along());
            assertTrue(s.facing() == s.along() || s.facing() == CellPlan.opposite(s.along()), "the panel faces along the chord");
            assertTrue(addresses.add(s.address()), "unique address " + s.address());
            assertTrue(cells.add(s.cell()), "unique cell " + s.cell());
            assertTrue(s.address().startsWith("172.31." + (s.chord() + 1) + "."), s.address());
            int host = Integer.parseInt(s.address().substring(s.address().lastIndexOf('.') + 1));
            assertTrue(host >= 16 && host <= 62, "in .16-.62 of the /26, past the players' .3-.14: " + host);
            assertEquals("172.31." + (s.chord() + 1) + ".1", s.gateway());
            assertEquals(s.k() % 2 == 1 ? s.along() : CellPlan.opposite(s.along()), s.facing(), "neighbours alternate their panels");
        }
    }

    @Test
    void provisioningIsDeterministicPerSeedAndDiffersBetweenSeeds() {
        CellPlan a = CellPlan.of(1L, ring(1L)), again = CellPlan.of(1L, ring(1L)), other = CellPlan.of(2L, ring(1L));
        assertEquals(a.coreDb(), again.coreDb());
        assertEquals(a.siteConfig("v3-store"), again.siteConfig("v3-store"));
        assertArrayEquals(a.sim(3, false, 7).key(), again.sim(3, false, 7).key());
        assertFalse(Arrays.equals(a.sim(3, false, 7).key(), other.sim(3, false, 7).key()), "another world, another key");
        assertFalse(Arrays.equals(a.siteSecret("v3-store"), other.siteSecret("v3-store")));
        assertFalse(Arrays.equals(a.sim(3, false, 7).key(), a.sim(3, false, 8).key()));
        assertFalse(Arrays.equals(a.sim(3, true, 1).key(), a.sim(3, false, 1).key()), "phone and chest SIMs differ");
        assertNotEquals(a.siteConfig("v3-store"), a.siteConfig("v4-store"));
    }

    @Test
    void theCoreDatabaseHoldsEverySiteAndEveryStoreSim() {
        long seed = 123456L;
        CellPlan plan = CellPlan.of(seed, ring(seed));
        String db = plan.coreDb();
        Pattern sub = Pattern.compile("^sub (\\d{15}) key=([0-9a-f]{32}) quota=0 used=0 revoked=0 label=(v\\d+-(phone|sim)\\d+) msisdn=(\\+1\\d{10})$");
        Map<String, String> numbers = new TreeMap<>();
        Set<String> blocks = new TreeSet<>();
        Pattern site = Pattern.compile("^site ([a-z0-9-]+) ([0-9a-f]{32})$");
        Map<String, String> subs = new TreeMap<>(), sites = new TreeMap<>();
        List<String> order = new ArrayList<>();
        int version = 0;
        for (String line : db.split("\n")) {
            if (line.startsWith("#")) continue;
            var m = sub.matcher(line);
            var s = site.matcher(line);
            if (m.matches()) {
                assertNull(subs.put(m.group(1), m.group(2)), "IMSI twice");
                assertNull(numbers.put(m.group(5), m.group(1)), "number twice");
                order.add(m.group(1));
            } else if (s.matches()) assertNull(sites.put(s.group(1), s.group(2)), "site twice");
            else if (line.equals("version 1")) version++;
            else if (line.startsWith("block ")) assertTrue(blocks.add(line.substring(6)), "block twice: " + line);
            else if (line.startsWith("ims ")) assertEquals("ims pool=10.250.0.0/22 pcscf=100.80.0.1 domain=" + plan.carrier + ".ecm net=100.80.0.1/32", line);
            else assertTrue(Set.of("carrier " + plan.carrier, "pool 100.80.0.0/22", "gateway 100.80.0.1", "dns 1.1.1.1", "mtu 1500")
                    .contains(line), "unexpected line '" + line + "'");
        }
        assertEquals(1, version);
        assertEquals(new ArrayList<>(new TreeSet<>(order)), order, "subscribers in cored's (sorted) order");
        assertEquals(10 * (CellPlan.PHONES + CellPlan.SIMS), subs.size());
        for (CellPlan.Sim m : plan.sims()) {
            assertTrue(m.imsi().startsWith(CellPlan.IMSI_PREFIX) && m.imsi().length() == 15, m.imsi());
            assertEquals(m.keyHex(), subs.get(m.imsi()), "the database holds the SIM's key: " + m.label());
            assertEquals(m.imsi(), numbers.get(m.msisdn()), "and its number: " + m.label());
            assertTrue(m.msisdn().startsWith("+1" + plan.areaCode(m.village()) + "555"), "a number in its village's block: " + m.msisdn());
        }
        assertEquals(10, blocks.size(), "a block (area code) per village");
        assertEquals(plan.number(1, true, 1), "+1" + plan.areaCode(1) + "5550100", "the first phone on display has line 0100");
        assertEquals(new TreeSet<>(plan.siteIds()), sites.keySet());
        for (String id : plan.siteIds()) {
            assertEquals(CellPlan.hex(plan.siteSecret(id)), sites.get(id));
            String conf = plan.siteConfig(id);
            assertTrue(conf.contains("\nsecret = " + sites.get(id) + "\n"), "the site's credential is the core's: " + id);
            assertTrue(conf.contains("\ncore = " + plan.coreAddress() + "\n"));
            assertTrue(conf.contains("\ncarrier = " + plan.carrier + "\n"));
            assertTrue(conf.contains("\ncell_id = " + plan.cellOf(id) + "\n"));
            for (String line : conf.split("\n"))
                if (!line.startsWith("#"))
                    assertTrue(Set.of("site_id", "secret", "carrier", "core", "cell_id", "profile", "channel", "power", "neighbours")
                            .contains(line.split(" = ")[0]), "a celld-site key: " + line);
        }
        assertTrue(plan.carrier.matches("[a-z0-9_-]{1,24}"));
        assertTrue(plan.channel >= 1 && plan.channel <= 6);
        assertTrue(plan.coreVillage >= 1 && plan.coreVillage <= 10);
    }

    @Test
    void neighboursAreTheRingOrderBothWaysOnOneChannel() {
        long seed = 99L;
        CellPlan plan = CellPlan.of(seed, ring(seed));
        List<String> ids = plan.siteIds();
        Set<Integer> cells = new HashSet<>();
        for (String id : ids) assertTrue(cells.add(plan.cellOf(id)), "unique cell " + id);
        for (String id : ids) {
            String[] nb = plan.neighbours(id).split(",");
            assertEquals(2, nb.length);
            for (String n : nb) {
                String[] cc = n.split(":");
                assertEquals(String.valueOf(plan.channel), cc[1], "one channel");
                String other = ids.stream().filter(o -> plan.cellOf(o) == Integer.parseInt(cc[0])).findFirst().orElseThrow();
                assertTrue(plan.neighbours(other).contains(plan.cellOf(id) + ":"), other + " lists " + id + " back");
            }
        }
        // The village is between the last site of the chord before it and the first of its own.
        int last = plan.spans(1).size();
        assertTrue(plan.neighbours("v3-store").startsWith((last > 0 ? 200 + last : 2) + ":"), plan.neighbours("v3-store"));
    }
}
