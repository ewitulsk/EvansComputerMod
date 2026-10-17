package com.example.evanscomputermod.computer;

import java.util.*;

/**
 * The Tech Village ring's DNS (docs/DNS.md), planned from the world seed: who runs which part
 * of the hierarchy, their addresses, and every file the servers start from.
 *
 * <p>Roles are dealt out from one seed-chosen base village B ({@link #baseVillage}) at fixed
 * offsets, so they always land in different villages and no single village is the whole of
 * DNS (docs/DNS.md "Who runs what"):
 * <pre>
 *   resolver            every village   100.(64+N).0.53  (or the ISP router, .1, with dns.resolverOnIspRouter)
 *   village zone vN.ring every village  100.(64+N).0.10  (the web server)
 *   IANA (root primary) B               .57
 *   root letters a-e    B+1,3,5,7,9     .54
 *   ecm. registry       B+2 (primary), B+7 (secondary)   .55
 *   ring. registry      B+4 (primary), B+9 (secondary)   .55
 *   registrar           B+6             .56
 * </pre>
 *
 * <p>Plain Java (no Minecraft types), unit-tested on both Minecraft versions ({@code DnsPlanTest}).
 */
public final class DnsPlan {
    /** Data center roles (computer identities are seed + village + role). */
    public static final String RESOLVER = "datacenter.dns", ROOT = "datacenter.root", REGISTRY = "datacenter.registry",
            REGISTRAR = "datacenter.registrar", IANA = "datacenter.iana";
    /** Last octet of each role on its village's data center LAN. */
    public static final int RESOLVER_HOST = 53, ROOT_HOST = 54, REGISTRY_HOST = 55, REGISTRAR_HOST = 56, IANA_HOST = 57;
    /** The village computers DNS adds to (WorldNetwork's roles; that class is 1.21.1 only). */
    public static final String WEB = "datacenter.web", ISP_ROUTER = "isp.router";
    public static final int VILLAGES = 10;
    /** Where the real internet is asked for names the in-game root does not know (through village 1's uplink). */
    public static final String FALLBACK = "1.1.1.1";
    /** EPP-lite port of the .ecm registry (registryd). */
    public static final int EPP_PORT = 700;
    public static final String LETTERS = "abcde";
    /** Root zone TTLs: delegations a day, negative answers 5 minutes (how soon a new TLD is seen). */
    public static final int ROOT_TTL = 86400, ROOT_NEGATIVE = 300;

    public final long seed;
    public final int base;
    public final boolean resolverOnIsp;
    /** The chat village (for chat.ring / repo.ring), 0 for none. */
    public final int chat;
    /** The carrier core village (core.ring), 0 for none. */
    public final int core;

    public DnsPlan(long seed, boolean resolverOnIsp, int chat, int core) {
        this.seed = seed;
        this.base = baseVillage(seed);
        this.resolverOnIsp = resolverOnIsp;
        this.chat = chat;
        this.core = core;
    }

    /** The base village B: random, fixed by the seed (its own salt, like the chat village's). */
    public static int baseVillage(long seed) {
        return 1 + (int) Math.floorMod(new SplittableRandom(seed ^ 0x444E53524F4F5453L).nextLong(), (long) VILLAGES);
    }

    /** Village B+{@code offset}, 1-10. */
    public int village(int offset) {
        return Math.floorMod(base - 1 + offset, VILLAGES) + 1;
    }

    public int offset(int village) {
        return Math.floorMod(village - base, VILLAGES);
    }

    public static String lan(int village, int host) {
        return "100." + (64 + village) + ".0." + host;
    }

    // ------------------------------------------------------------------ roles

    public int iana() {
        return village(0);
    }

    /** Root letter of village {@code v} ('a'-'e') or 0. */
    public char letter(int v) {
        int o = offset(v);
        return o % 2 == 1 ? LETTERS.charAt(o / 2) : 0;
    }

    /** The village running root letter {@code l} ('a'-'e'). */
    public int letterVillage(char l) {
        return village(1 + 2 * LETTERS.indexOf(l));
    }

    public int ecmPrimary() {
        return village(2);
    }

    public int ecmSecondary() {
        return village(7);
    }

    public int ringPrimary() {
        return village(4);
    }

    public int ringSecondary() {
        return village(9);
    }

    public int registrar() {
        return village(6);
    }

    /** The extra data center roles of village {@code v} beside its web server (resolver first). */
    public List<String> roles(int v) {
        List<String> out = new ArrayList<>();
        if (!resolverOnIsp) out.add(RESOLVER);
        if (v == iana()) out.add(IANA);
        if (letter(v) != 0) out.add(ROOT);
        if (v == ecmPrimary() || v == ecmSecondary() || v == ringPrimary() || v == ringSecondary()) out.add(REGISTRY);
        if (v == registrar()) out.add(REGISTRAR);
        return out;
    }

    public static boolean isDnsRole(String role) {
        return role.equals(RESOLVER) || role.equals(ROOT) || role.equals(REGISTRY) || role.equals(REGISTRAR) || role.equals(IANA);
    }

    public static int host(String role) {
        return switch (role) {
            case RESOLVER -> RESOLVER_HOST;
            case ROOT -> ROOT_HOST;
            case REGISTRY -> REGISTRY_HOST;
            case REGISTRAR -> REGISTRAR_HOST;
            case IANA -> IANA_HOST;
            default -> throw new IllegalArgumentException(role);
        };
    }

    /** What village {@code v}'s hierarchy computers do, in words (techvillage info, the website). */
    public List<String> describe(int v) {
        List<String> out = new ArrayList<>();
        out.add("recursive resolver " + resolver(v) + (resolverOnIsp ? " (on the ISP router)" : ""));
        out.add("village zone v" + v + ".ring on the web server " + lan(v, 10));
        if (v == iana()) out.add("IANA: the root zone's hidden primary and TLD applications, " + lan(v, IANA_HOST) + " (iana.ring)");
        if (letter(v) != 0) out.add("root server " + letter(v) + ".root-servers.ring, " + lan(v, ROOT_HOST));
        if (v == ecmPrimary()) out.add("the .ecm registry (primary, EPP port " + EPP_PORT + "), " + lan(v, REGISTRY_HOST) + " (ns1.ecm.ring)");
        if (v == ecmSecondary()) out.add("the .ecm registry's secondary, " + lan(v, REGISTRY_HOST) + " (ns2.ecm.ring)");
        if (v == ringPrimary()) out.add("the .ring registry (primary), " + lan(v, REGISTRY_HOST) + " (ns1.ring.ring)");
        if (v == ringSecondary()) out.add("the .ring registry's secondary, " + lan(v, REGISTRY_HOST) + " (ns2.ring.ring)");
        if (v == registrar()) out.add("the registrar: http://nic.ring/, whois.nic.ring, " + lan(v, REGISTRAR_HOST));
        return out;
    }

    // ------------------------------------------------------------------ addresses

    /** The resolver village {@code v}'s DHCP hands out. */
    public String resolver(int v) {
        return resolverOnIsp ? lan(v, 1) : lan(v, RESOLVER_HOST);
    }

    public String rootServer(char l) {
        return lan(letterVillage(l), ROOT_HOST);
    }

    public String ianaAddress() {
        return lan(iana(), IANA_HOST);
    }

    public String ecmPrimaryAddress() {
        return lan(ecmPrimary(), REGISTRY_HOST);
    }

    public String ecmSecondaryAddress() {
        return lan(ecmSecondary(), REGISTRY_HOST);
    }

    public String ringPrimaryAddress() {
        return lan(ringPrimary(), REGISTRY_HOST);
    }

    public String ringSecondaryAddress() {
        return lan(ringSecondary(), REGISTRY_HOST);
    }

    public String registrarAddress() {
        return lan(registrar(), REGISTRAR_HOST);
    }

    /** The registrar's EPP-lite login at the .ecm registry: fixed by the seed. */
    public String registrarSecret() {
        SplittableRandom r = new SplittableRandom(seed ^ 0x4550504C49544531L);
        StringBuilder s = new StringBuilder();
        for (int i = 0; i < 4; i++) s.append(String.format("%08x", r.nextInt()));
        return s.toString();
    }

    public static final String REGISTRAR_ID = "nic-ring";

    // ------------------------------------------------------------------ files

    /** /etc/named/root.hints: where resolvers start. */
    public String rootHints() {
        StringBuilder s = new StringBuilder("; The root servers of the Tech Village ring (docs/DNS.md)\n");
        for (char l : LETTERS.toCharArray()) s.append(".                      3600000 NS ").append(l).append(".root-servers.ring.\n");
        for (char l : LETTERS.toCharArray())
            s.append(l).append(".root-servers.ring.    3600000 A  ").append(rootServer(l)).append('\n');
        return s.toString();
    }

    private static String soa(String mname, int serial, int refresh, int retry, int expire, int minimum) {
        return "@ SOA " + mname + " hostmaster.nic.ring. " + serial + " " + refresh + " " + retry + " " + expire + " " + minimum + "\n";
    }

    /** The root zone (IANA's hidden primary; the letters load it by transfer). */
    public String rootZone() {
        StringBuilder s = new StringBuilder("; The root zone of the Tech Village ring, kept by IANA (village " + iana() + ").\n");
        s.append("; ianad adds TLDs it approves; the root letters copy it by NOTIFY + AXFR.\n");
        s.append("$ORIGIN .\n$TTL ").append(ROOT_TTL).append('\n');
        s.append(soa("a.root-servers.ring.", 1, 300, 60, 604800, ROOT_NEGATIVE));
        for (char l : LETTERS.toCharArray()) s.append("@ NS ").append(l).append(".root-servers.ring.\n");
        for (char l : LETTERS.toCharArray()) s.append(l).append(".root-servers.ring. A ").append(rootServer(l)).append('\n');
        s.append("ring. NS ns1.ring.ring.\nring. NS ns2.ring.ring.\n");
        s.append("ns1.ring.ring. A ").append(ringPrimaryAddress()).append('\n');
        s.append("ns2.ring.ring. A ").append(ringSecondaryAddress()).append('\n');
        s.append("ecm. NS ns1.ecm.ring.\necm. NS ns2.ecm.ring.\n");
        s.append("ns1.ecm.ring. A ").append(ecmPrimaryAddress()).append('\n');
        s.append("ns2.ecm.ring. A ").append(ecmSecondaryAddress()).append('\n');
        return s.toString();
    }

    /** The ring. zone: every village's delegation and the ring-wide names. */
    public String ringZone() {
        StringBuilder s = new StringBuilder("; ring.: the Tech Village infrastructure TLD (the .ring registry, village " + ringPrimary() + ")\n");
        s.append("$ORIGIN ring.\n$TTL 3600\n");
        s.append(soa("ns1.ring.ring.", 1, 300, 60, 604800, 60));
        s.append("@ NS ns1.ring\n@ NS ns2.ring\n");
        s.append("ns1.ring A ").append(ringPrimaryAddress()).append('\n');
        s.append("ns2.ring A ").append(ringSecondaryAddress()).append('\n');
        for (char l : LETTERS.toCharArray()) s.append(l).append(".root-servers A ").append(rootServer(l)).append('\n');
        s.append("ns1.ecm A ").append(ecmPrimaryAddress()).append('\n');
        s.append("ns2.ecm A ").append(ecmSecondaryAddress()).append('\n');
        s.append("nic A ").append(registrarAddress()).append('\n');
        s.append("whois.nic CNAME nic\n");
        s.append("iana A ").append(ianaAddress()).append('\n');
        if (chat > 0) {
            s.append("chat A ").append(lan(chat, 20)).append('\n');
            s.append("repo A ").append(lan(chat, VillageRepo.HOST)).append('\n');
        }
        if (core > 0) s.append("core A ").append(lan(core, CellPlan.CORE_HOST)).append('\n');
        for (int v = 1; v <= VILLAGES; v++) {
            s.append("v").append(v).append(" NS ns.v").append(v).append('\n');
            s.append("ns.v").append(v).append(" A ").append(lan(v, 10)).append('\n');
        }
        return s.toString();
    }

    /** vN.ring, served by village N's web server. */
    public String villageZone(int v) {
        StringBuilder s = new StringBuilder("; v" + v + ".ring: Tech Village " + v + " (its web server is authoritative)\n");
        s.append("$ORIGIN v").append(v).append(".ring.\n$TTL 300\n");
        s.append(soa("ns", 1, 300, 60, 604800, 60));
        s.append("@ NS ns\n");
        s.append("ns A ").append(lan(v, 10)).append('\n');
        s.append("web A ").append(lan(v, 10)).append('\n');
        s.append("www CNAME web\n");
        s.append("isp A ").append(lan(v, 1)).append('\n');
        s.append("dns A ").append(resolver(v)).append('\n');
        if (v == chat) {
            s.append("chat A ").append(lan(v, 20)).append('\n');
            s.append("repo A ").append(lan(v, VillageRepo.HOST)).append('\n');
        }
        if (v == core) s.append("core A ").append(lan(v, CellPlan.CORE_HOST)).append('\n');
        s.append("store A ").append(lan(v, CellPlan.STORE_HOST)).append('\n');
        if (letter(v) != 0) s.append("root A ").append(lan(v, ROOT_HOST)).append('\n');
        if (roles(v).contains(REGISTRY)) s.append("registry A ").append(lan(v, REGISTRY_HOST)).append('\n');
        if (v == registrar()) s.append("registrar A ").append(lan(v, REGISTRAR_HOST)).append('\n');
        if (v == iana()) s.append("iana A ").append(lan(v, IANA_HOST)).append('\n');
        return s.toString();
    }

    /** The empty ecm. zone the registry starts from (registryd adds the domains). */
    public String ecmZone() {
        return "; ecm.: the public TLD (the .ecm registry, village " + ecmPrimary() + "). registryd owns this file.\n"
                + "$ORIGIN ecm.\n$TTL 3600\n"
                + soa("ns1.ecm.ring.", 1, 300, 60, 604800, 60)
                + "@ NS ns1.ecm.ring.\n@ NS ns2.ecm.ring.\n";
    }

    /** /etc/named.conf of the resolver (the .53 computer, or the ISP router with the flag on). */
    public String resolverConf() {
        return "# The village's recursive resolver (docs/DNS.md)\nrecursion yes\n"
                + "allow 127.0.0.0/8 10.0.0.0/8 100.64.0.0/10 172.16.0.0/12 192.168.0.0/16\n"
                + "root-hints /etc/named/root.hints\nfallback " + FALLBACK + "\n";
    }

    /** Every file of village {@code v}'s computer {@code role} for DNS ({@code role} may be the web server or the ISP). */
    public Map<String, String> files(int v, String role) {
        Map<String, String> f = new LinkedHashMap<>();
        switch (role) {
            case WEB -> {
                f.put("etc/named.conf", "# v" + v + ".ring, authoritative (docs/DNS.md)\nzone v" + v + ".ring primary /etc/named/v" + v + ".ring.zone\n");
                f.put("etc/named/v" + v + ".ring.zone", villageZone(v));
            }
            case ISP_ROUTER -> {
                if (resolverOnIsp) {
                    f.put("etc/named.conf", resolverConf());
                    f.put("etc/named/root.hints", rootHints());
                }
            }
            case RESOLVER -> {
                f.put("etc/named.conf", resolverConf());
                f.put("etc/named/root.hints", rootHints());
            }
            case IANA -> {
                f.put("etc/named.conf", "# IANA: the hidden primary of the root zone; the letters transfer it\nzone . primary /etc/named/root.zone\n");
                f.put("etc/named/root.zone", rootZone());
                f.put("etc/ianad.conf", "# ianad: TLD applications (docs/DNS.md \"Adding TLDs\")\nzone /etc/named/root.zone\nport 80\n");
            }
            case ROOT -> f.put("etc/named.conf", "# root server " + letter(v) + ".root-servers.ring: a secondary of IANA\nzone . secondary " + ianaAddress() + "\n");
            case REGISTRY -> {
                StringBuilder c = new StringBuilder("# The registry servers of village " + v + "\n");
                if (v == ecmPrimary()) {
                    c.append("zone ecm primary /etc/named/ecm.zone notify ").append(ecmSecondaryAddress()).append('\n');
                    f.put("etc/named/ecm.zone", ecmZone());
                    f.put("etc/registryd.conf", "# registryd: the .ecm registry (EPP-lite on port " + EPP_PORT + ")\nzone ecm /etc/named/ecm.zone\nport "
                            + EPP_PORT + "\nregistrar " + REGISTRAR_ID + " " + registrarSecret() + "\n");
                }
                if (v == ecmSecondary()) c.append("zone ecm secondary ").append(ecmPrimaryAddress()).append('\n');
                if (v == ringPrimary()) {
                    c.append("zone ring primary /etc/named/ring.zone notify ").append(ringSecondaryAddress()).append('\n');
                    f.put("etc/named/ring.zone", ringZone());
                }
                if (v == ringSecondary()) c.append("zone ring secondary ").append(ringPrimaryAddress()).append('\n');
                f.put("etc/named.conf", c.toString());
            }
            case REGISTRAR -> f.put("etc/registrard.conf", "# registrard: the registrar (http://nic.ring/, whois port 43)\nregistry "
                    + ecmPrimaryAddress() + " " + EPP_PORT + "\nid " + REGISTRAR_ID + "\nsecret " + registrarSecret() + "\nport 80\n");
            default -> {}
        }
        return f;
    }

    /** services.cfg lines a role adds for DNS. */
    public String services(int v, String role) {
        return switch (role) {
            case WEB -> "named &\n";
            case ISP_ROUTER -> resolverOnIsp ? "named &\n" : "";
            // No sshd on the DNS computers: 21 fewer processes on the server, and nothing there
            // needs a remote shell.
            case RESOLVER, ROOT -> "named &\n";
            case IANA -> "named &\nianad &\n";
            case REGISTRY -> v == ecmPrimary() ? "named &\nregistryd &\n" : "named &\n";
            case REGISTRAR -> "registrard &\n";
            default -> "";
        };
    }

    /** A data center server's network.cfg: its address, the default route and the village's resolver. */
    public String serverNetwork(int v, int host) {
        return "iface eth1 " + lan(v, host) + "/24\nroute default via " + lan(v, 1) + " dev eth1\ndns " + resolver(v) + "\n";
    }

    /** /etc/resolv.conf of a village server (search vN.ring). */
    public String resolvConf(int v) {
        return "# Written by the kernel from DHCP / resolvectl. Edit it and it is yours.\nnameserver " + resolver(v) + "\nsearch v" + v + ".ring\n";
    }

    /** {@code /ecm dns roles}: the whole map. */
    public List<String> map() {
        List<String> out = new ArrayList<>();
        out.add("Base village B = " + base + (resolverOnIsp ? " (resolvers on the ISP routers)" : ""));
        out.add("IANA (root primary): village " + iana() + " " + ianaAddress());
        for (char l : LETTERS.toCharArray())
            out.add(l + ".root-servers.ring: village " + letterVillage(l) + " " + rootServer(l));
        out.add("ecm. registry: village " + ecmPrimary() + " " + ecmPrimaryAddress() + " (primary, EPP " + EPP_PORT + "), village "
                + ecmSecondary() + " " + ecmSecondaryAddress() + " (secondary)");
        out.add("ring. registry: village " + ringPrimary() + " " + ringPrimaryAddress() + " (primary), village " + ringSecondary() + " "
                + ringSecondaryAddress() + " (secondary)");
        out.add("Registrar: village " + registrar() + " " + registrarAddress() + " (http://nic.ring/, whois.nic.ring)");
        out.add("Resolvers: " + (resolverOnIsp ? "100.(64+N).0.1 on each ISP router" : "100.(64+N).0.53 in every village"));
        return out;
    }
}
