package com.example.evanscomputermod.computer;

import java.nio.charset.StandardCharsets;
import java.util.*;

/**
 * The Tech Village carrier network (Cellular v2 phase 5, docs/gui/CELLULAR_V2.md "As built:
 * phase 5"), planned from the world seed and the fiber ring alone, so every piece agrees
 * before any chunk exists: one carrier, one core, a cell site in every village's Phone
 * Store, cell sites along every fiber chord, and the SIMs of every Phone Store (the phones
 * on display and the chest), all provisioned in the core's subscriber database.
 *
 * <p>Plain Java (no Minecraft types), so placement and provisioning are unit-tested on both
 * Minecraft versions ({@code CellPlanTest}).
 *
 * <h2>Span sites</h2>
 * A chord's sites sit at equal horizontal steps of at most {@link #SPACING} blocks between
 * the two masts. The link budget is not what limits the spacing: an LTE site (43 dBm, a
 * 14 dBi panel) still clears the robust CQI kilometres away in the open. A radio only works
 * while its chunk ticks, and a server ticks the chunks within its simulation distance of a
 * player (10 chunks by default): at most 256 blocks apart, the midpoint between two sites is
 * at most 8 chunks from both, so both are on the air there and the phone hands over between
 * them instead of losing the cell.
 *
 * <p>Each span site is a column beside the path: a Fiber Patch Panel against the span (a tap,
 * as players tap the ring) at the fiber's height, a network cable riser to the computer on
 * the ground, an Advanced SDR beside the computer, and a coax run up beside a lattice mast to
 * an LTE Sector Antenna. Its address is in the chord's link subnet {@code 172.31.N.0/26}: the ISPs
 * keep .1 and .2, players' taps .3-.14, the cell sites take .16 onwards.
 */
public final class CellPlan {
    /** Largest step between neighbouring sites along a chord, in blocks (see the class comment). */
    public static final int SPACING = 256;
    /** Planned step: below {@link #SPACING}, so a site moved off a blocked spot still keeps within it. */
    public static final int NOMINAL = 224;
    /** Coax column height of a span site: the panel antenna is this many blocks above its computer. */
    public static final int MAST = 10;
    public static final String POOL = "100.80.0.0/22", GATEWAY = "100.80.0.1", DNS = "1.1.1.1";
    public static final String POOL_NET = "100.80.0.0", POOL_LEN = "22";
    /**
     * Telephony (docs/TELEPHONY.md "Tech Villages"): the phones' IMS bearer pool (it reaches only
     * the core, where imsd, smscd, mmscd and mrfd run beside cored), the exchange every village's
     * block uses, and the first line handed out in a block (as cored's {@code FIRST_LINE}).
     */
    public static final String IMS_POOL = "10.250.0.0/22", EXCHANGE = "555";
    public static final int FIRST_LINE = 100;
    /** Area codes a village may get (valid NANP area codes, no N11): each village its own, picked by the seed. */
    public static final List<String> AREA_CODES = List.of("214", "312", "415", "503", "617", "702", "808", "919", "248", "360",
            "425", "512", "602", "713", "804", "971", "206", "305", "404", "720");
    /** Phones on display and SIM cards in the chest of every Phone Store. */
    public static final int PHONES = 4, SIMS = 27;
    /** MCC 001 MNC 01, the test network (cored's default {@code imsi_prefix}). */
    public static final String IMSI_PREFIX = "00101";
    /** Last octet of the core (in its village's data center LAN) and of the Phone Store's site. */
    public static final int CORE_HOST = 30, STORE_HOST = 40;
    /** First host of the span sites in a chord's link subnet (.1/.2 ISPs, .3-.14 taps). */
    public static final int FIRST_SPAN_HOST = 16, LINK_PREFIX = 26;
    /** The largest number of span sites a chord's /26 holds (.16-.62). */
    public static final int MAX_SPANS = 47;
    /** Carrier names a world may get (lower case: the SIM's carrier field). */
    public static final List<String> CARRIERS = List.of("technet", "ringtel", "fibercell", "villanet", "meshmobile",
            "oakwave", "emberlink", "skyring");
    /** Horizontal Direction 3D data values (as {@link FiberChords}). */
    public static final int NORTH = 2, SOUTH = 3, WEST = 4, EAST = 5;

    /**
     * One cell site along a chord. Column {@code (x, z)} holds the computer (on the ground),
     * the riser and the tap panel at {@code panelY} (against path block {@code index}); the
     * SDR is one step {@code along}, the coax column one step {@code out} from the SDR (away
     * from the path) with the LTE panel on top facing {@code facing} (along the chord), and the
     * lattice mast one more step out, beside the panel and never in its beam.
     */
    public record SpanSite(int chord, int k, int index, int x, int panelY, int z, int out, int along, int facing,
                           int cell, String id, String address, String gateway) {
        public int village() {
            return chord + 1;
        }

        /** Provisioning role of the site's computer (in village {@link #village()}). */
        public String role() {
            return spanRole(k);
        }

        /** The columns the site occupies: computer, SDR, coax, mast ({x, z} each). */
        public int[][] columns() {
            return CellPlan.columns(x, z, out, along);
        }
    }

    /** A provisioned SIM: in a Phone Store's phone ({@code phone}) or its chest, {@code slot} from 1. */
    public record Sim(String imsi, byte[] key, String label, int village, boolean phone, int slot, String msisdn) {
        public String keyHex() {
            return hex(key);
        }
    }

    public final long seed;
    public final String carrier;
    public final int coreVillage, channel, villages;
    private final List<List<SpanSite>> spans = new ArrayList<>();

    private CellPlan(long seed, FiberChords ring) {
        this.seed = seed;
        this.villages = ring.chordCount();
        this.carrier = carrier(seed);
        this.coreVillage = coreVillage(seed, villages);
        this.channel = channel(seed);
        Set<Long> columns = new HashSet<>();
        Set<Long> blocks = new HashSet<>();
        for (int c = 0; c < villages; c++)
            for (int[] p : ring.path(c)) {
                columns.add(column(p[0], p[2]));
                blocks.add(FiberChords.pack(p[0], p[1], p[2]));
            }
        for (int c = 0; c < villages; c++) spans.add(List.copyOf(placeChord(ring, c, columns, blocks)));
    }

    public static CellPlan of(long seed, FiberChords ring) {
        return new CellPlan(seed, ring);
    }

    // ------------------------------------------------------------------ seed choices

    /** The village whose data center runs the carrier core: random, fixed by the seed. */
    public static int coreVillage(long seed, int villages) {
        return 1 + (int) Math.floorMod(new SplittableRandom(seed ^ 0x434F52454E455457L).nextLong(), (long) villages);
    }

    public static String carrier(long seed) {
        return CARRIERS.get((int) Math.floorMod(new SplittableRandom(seed ^ 0x4341525249455221L).nextLong(), (long) CARRIERS.size()));
    }

    /** The LTE channel every site of the carrier uses (1-6). */
    public static int channel(long seed) {
        return 1 + (int) Math.floorMod(new SplittableRandom(seed ^ 0x4348414E4E454C31L).nextLong(), 6L);
    }

    /** The carrier's display name, e.g. "TechNet" (status bar shows the lower-case carrier id). */
    public static String displayName(String carrier) {
        return switch (carrier) {
            case "technet" -> "TechNet";
            case "ringtel" -> "RingTel";
            case "fibercell" -> "FiberCell";
            case "villanet" -> "VillaNet";
            case "meshmobile" -> "MeshMobile";
            case "oakwave" -> "OakWave";
            case "emberlink" -> "EmberLink";
            case "skyring" -> "SkyRing";
            default -> carrier;
        };
    }

    /**
     * The DNS server the core hands its phones: the core village's resolver once the ring has
     * DNS ({@link DnsPlan}, set by {@code WorldNetwork}), {@link #DNS} before.
     */
    public String dns = DNS;

    public String displayName() {
        return displayName(carrier);
    }

    // ------------------------------------------------------------------ addresses

    public static String villageLan(int village, int host) {
        return "100." + (64 + village) + ".0." + host;
    }

    /** The core's address on its village's data center LAN. */
    public String coreAddress() {
        return villageLan(coreVillage, CORE_HOST);
    }

    public static String linkAddress(int chord, int host) {
        return "172.31." + (chord + 1) + "." + host;
    }

    // ------------------------------------------------------------------ sites

    public List<SpanSite> spans(int chord) {
        return spans.get(chord);
    }

    public List<SpanSite> allSpans() {
        List<SpanSite> out = new ArrayList<>();
        for (var l : spans) out.addAll(l);
        return out;
    }

    public SpanSite span(int chord, int k) {
        for (SpanSite s : spans.get(chord)) if (s.k == k) return s;
        return null;
    }

    public static String spanRole(int k) {
        return String.format("span%02d.site", k);
    }

    /** {@code k} of a span-site role, or -1. */
    public static int spanK(String role) {
        if (!role.startsWith("span") || !role.endsWith(".site")) return -1;
        try {
            return Integer.parseInt(role.substring(4, role.length() - 5));
        } catch (NumberFormatException e) {
            return -1;
        }
    }

    public static final String STORE_ROLE = "store.site";

    public static String storeSiteId(int village) {
        return "v" + village + "-store";
    }

    public static String spanSiteId(int chord, int k) {
        return "c" + (chord + 1) + "-s" + k;
    }

    /** Every site id in ring order: village 1, chord 1's span sites, village 2, ... */
    public List<String> siteIds() {
        List<String> out = new ArrayList<>();
        for (int v = 1; v <= villages; v++) {
            out.add(storeSiteId(v));
            for (SpanSite s : spans.get(v - 1)) out.add(s.id);
        }
        return out;
    }

    /** Cell id of a site id (village sites are their village number, span sites 100 * N + k). */
    public int cellOf(String siteId) {
        if (siteId.startsWith("v")) return Integer.parseInt(siteId.substring(1, siteId.indexOf('-')));
        String[] p = siteId.substring(1).split("-s");
        return 100 * Integer.parseInt(p[0]) + Integer.parseInt(p[1]);
    }

    /** The site's 16-byte credential, from the seed (server-only, like the world seed). */
    public byte[] siteSecret(String siteId) {
        return derive("ECM-TECHNET-SITE", siteId);
    }

    /** Neighbour cells of a site in ring order (the previous and the next site), "cell:channel,...". */
    public String neighbours(String siteId) {
        List<String> ring = siteIds();
        int i = ring.indexOf(siteId);
        if (i < 0) throw new IllegalArgumentException("no site " + siteId);
        String prev = ring.get((i + ring.size() - 1) % ring.size()), next = ring.get((i + 1) % ring.size());
        return cellOf(prev) + ":" + channel + "," + cellOf(next) + ":" + channel;
    }

    /** /etc/celld-site.toml of a site: what {@code celld-site config set} writes for these values. */
    public String siteConfig(String siteId) {
        return "# celld-site: this computer's cell site (docs/gui/CELLULAR_V2.md). Edit with\n"
                + "# `celld-site config set <key> <value>` or the Cell Site app, then `celld-site restart`.\n"
                + "site_id = " + siteId + "\n"
                + "secret = " + hex(siteSecret(siteId)) + "\n"
                + "carrier = " + carrier + "\n"
                + "core = " + coreAddress() + "\n"
                + "cell_id = " + cellOf(siteId) + "\n"
                + "profile = lte\n"
                + "channel = " + channel + "\n"
                + "power = auto\n"
                + "neighbours = " + neighbours(siteId) + "\n";
    }

    // ------------------------------------------------------------------ SIMs

    public static String imsi(long seed, int village, boolean phone, int slot) {
        int salt = (int) Math.floorMod(new SplittableRandom(seed ^ 0x494D534953414C54L).nextLong(), 100000L);
        return IMSI_PREFIX + String.format("%02d%d%02d%05d", village, phone ? 0 : 1, slot, salt);
    }

    public Sim sim(int village, boolean phone, int slot) {
        String imsi = imsi(seed, village, phone, slot);
        String label = "v" + village + (phone ? "-phone" : "-sim") + slot;
        return new Sim(imsi, derive("ECM-TECHNET-SIM", imsi), label, village, phone, slot, number(village, phone, slot));
    }

    // ------------------------------------------------------------------ numbers

    /** Village {@code v}'s area code: distinct per village (the list shuffled by the seed, then numbered past its end). */
    public String areaCode(int village) {
        List<String> codes = new ArrayList<>(AREA_CODES);
        Collections.shuffle(codes, new Random(seed ^ 0x41524541434F4445L));
        if (village - 1 < codes.size()) return codes.get(village - 1);
        // more villages than listed codes: 2v0-style codes never in the list (2-9 first digit, not N11)
        return String.valueOf(200 + 10 * (village - codes.size()) + 3);
    }

    /** Village {@code v}'s number block, as cored writes it ({@code block 214-555}). */
    public String block(int village) {
        return areaCode(village) + "-" + EXCHANGE;
    }

    /** The Phone Store SIM's number: the phones on display get lines 0100-0103, the chest's SIMs 0104 onwards. */
    public String number(int village, boolean phone, int slot) {
        int line = FIRST_LINE + (phone ? slot - 1 : PHONES + slot - 1);
        return String.format("+1%s%s%04d", areaCode(village), EXCHANGE, line);
    }

    /** The carrier's SIP domain (cored's default for an {@code ims} line). */
    public String imsDomain() {
        return carrier + ".ecm";
    }

    /** Every SIM of every Phone Store, village by village: the phones on display, then the chest. */
    public List<Sim> sims() {
        List<Sim> out = new ArrayList<>();
        for (int v = 1; v <= villages; v++) {
            for (int i = 1; i <= PHONES; i++) out.add(sim(v, true, i));
            for (int i = 1; i <= SIMS; i++) out.add(sim(v, false, i));
        }
        return out;
    }

    /**
     * The core's /etc/cored.db as {@code cored init <carrier> 100.80.0.0/22 --dns 1.1.1.1},
     * {@code cored site-add} for every site and {@code cored add <imsi> <key> 0 <label>} for
     * every Phone Store SIM would leave it, in cored's own order (sites and subscribers sorted).
     */
    public String coreDb() {
        StringBuilder s = new StringBuilder();
        s.append("# ecm-cell subscriber database (owned by cored; edit with the Carrier Console or cored's admin commands)\n");
        s.append("version 1\ncarrier ").append(carrier).append("\npool ").append(POOL).append("\ngateway ").append(GATEWAY)
                .append("\ndns ").append(dns).append("\nmtu 1500\n");
        for (int v = 1; v <= villages; v++) s.append("block ").append(block(v)).append('\n');
        s.append("ims pool=").append(IMS_POOL).append(" pcscf=").append(GATEWAY).append(" domain=").append(imsDomain())
                .append(" net=").append(GATEWAY).append("/32\n");
        TreeMap<String, String> sites = new TreeMap<>();
        for (String id : siteIds()) sites.put(id, hex(siteSecret(id)));
        sites.forEach((id, secret) -> s.append("site ").append(id).append(' ').append(secret).append('\n'));
        TreeMap<String, Sim> subs = new TreeMap<>();
        for (Sim m : sims()) subs.put(m.imsi, m);
        for (Sim m : subs.values())
            s.append("sub ").append(m.imsi).append(" key=").append(m.keyHex()).append(" quota=0 used=0 revoked=0 label=")
                    .append(m.label).append(" msisdn=").append(m.msisdn).append('\n');
        return s.toString();
    }

    /** HMAC-SHA256(seed-derived key, label || 0 || name)[0..16). */
    private byte[] derive(String label, String name) {
        try {
            var mac = javax.crypto.Mac.getInstance("HmacSHA256");
            byte[] k = new byte[16];
            for (int i = 0; i < 8; i++) k[i] = (byte) (seed >>> (56 - 8 * i));
            byte[] tag = "ECM-TECHNET".getBytes(StandardCharsets.US_ASCII);
            System.arraycopy(tag, 0, k, 8, 8);
            mac.init(new javax.crypto.spec.SecretKeySpec(k, "HmacSHA256"));
            mac.update(label.getBytes(StandardCharsets.US_ASCII));
            mac.update((byte) 0);
            return Arrays.copyOf(mac.doFinal(name.getBytes(StandardCharsets.US_ASCII)), 16);
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }

    public static String hex(byte[] b) {
        StringBuilder s = new StringBuilder();
        for (byte x : b) s.append(String.format("%02x", x & 0xff));
        return s.toString();
    }

    // ------------------------------------------------------------------ placement

    private static long column(int x, int z) {
        return FiberChords.pack(x, 0, z);
    }

    private static double horizontal(int[] a, int[] b) {
        return Math.hypot(a[0] - b[0], a[2] - b[2]);
    }

    /** Sites of chord {@code c} at {@code n} equal steps of at most {@link #SPACING}. */
    private List<SpanSite> placeChord(FiberChords ring, int c, Set<Long> columns, Set<Long> blocks) {
        int[][] path = ring.path(c);
        int[] start = path[0], end = path[path.length - 1];
        double total = horizontal(start, end);
        int n = Math.min(MAX_SPANS, Math.max(0, (int) Math.ceil(total / NOMINAL) - 1));
        int dx = end[0] - start[0], dz = end[2] - start[2];
        int along = Math.abs(dx) >= Math.abs(dz) ? (dx >= 0 ? EAST : WEST) : (dz >= 0 ? SOUTH : NORTH);
        // The drift side (the minor axis the path steps along) and the side away from it.
        int drift = Math.abs(dx) >= Math.abs(dz) ? (dz >= 0 ? SOUTH : NORTH) : (dx >= 0 ? EAST : WEST);
        int away = opposite(drift);
        List<SpanSite> out = new ArrayList<>();
        int from = 0;
        int[] last = start;
        for (int k = 1; k <= n; k++) {
            double target = total * k / (n + 1);
            int i0 = from;
            while (i0 < path.length - 1 && horizontal(path[i0], start) < target) i0++;
            SpanSite site = null;
            for (int d = 0; d <= 32 && site == null; d++)
                for (int sign : new int[] {1, -1}) {
                    int i = i0 + d * sign;
                    if (d == 0 && sign < 0) continue;
                    if (i < 48 || i > path.length - 49 || horizontal(path[i], last) > SPACING - 2) continue;
                    for (int out2 : new int[] {away, drift}) {
                        site = tryPlace(c, k, path, i, along, out2, columns, blocks);
                        if (site != null) break;
                    }
                    if (site != null) break;
                }
            if (site == null) continue;   // no room for this one (tested: never on the planned rings)
            out.add(site);
            from = site.index;
            last = path[site.index];
        }
        return out;
    }

    private SpanSite tryPlace(int c, int k, int[][] path, int i, int along, int out, Set<Long> columns, Set<Long> blocks) {
        int[] p = path[i], o = FiberChords.step(out), a = FiberChords.step(along);
        int x = p[0] + o[0], z = p[2] + o[1];
        int[][] cols = columns(x, z, out, along);
        int cx = x >> 4, cz = z >> 4;
        for (int[] col : cols) {
            if (columns.contains(column(col[0], col[1]))) return null;
            if ((col[0] >> 4) != cx || (col[1] >> 4) != cz) return null;
        }
        // The tap panel touches exactly one path block: the one beside it.
        if (blocks.contains(FiberChords.pack(x - a[0], p[1], z - a[1]))) return null;
        int facing = k % 2 == 1 ? along : opposite(along);
        int host = FIRST_SPAN_HOST + k - 1;
        return new SpanSite(c, k, i, x, p[1], z, out, along, facing, 100 * (c + 1) + k, spanSiteId(c, k),
                linkAddress(c, host), linkAddress(c, 1));
    }

    /** Computer, SDR, coax and mast columns of a site whose computer stands at (x, z). */
    public static int[][] columns(int x, int z, int out, int along) {
        int[] o = FiberChords.step(out), a = FiberChords.step(along);
        return new int[][] {{x, z}, {x + a[0], z + a[1]}, {x + a[0] + o[0], z + a[1] + o[1]},
                {x + a[0] + 2 * o[0], z + a[1] + 2 * o[1]}};
    }

    public static int opposite(int dir) {
        return switch (dir) {
            case NORTH -> SOUTH;
            case SOUTH -> NORTH;
            case WEST -> EAST;
            case EAST -> WEST;
            default -> throw new IllegalArgumentException("not horizontal: " + dir);
        };
    }
}
