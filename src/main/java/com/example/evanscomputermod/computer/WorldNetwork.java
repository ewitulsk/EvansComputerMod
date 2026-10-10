package com.example.evanscomputermod.computer;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;

import net.minecraft.core.*;
import net.minecraft.nbt.*;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.tags.BiomeTags;
import net.minecraft.tags.TagKey;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.level.saveddata.SavedData;

import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.*;

/**
 * Durable village identities, the long-distance fiber ring and last-known cable blocks.
 *
 * <p>Village sites, their style and their fiber endpoints are fixed at plan time, before
 * any village chunk exists: the ISP mast stands on the site's (x, z) and the start
 * piece's height comes from the generator's {@code WORLD_SURFACE_WG} estimate, exactly
 * as JigsawStructure computes it. The fiber paths between neighbouring endpoints are
 * derived from those endpoints ({@link FiberChords}).
 */
public final class WorldNetwork extends SavedData {
    public static final List<String> STYLES = List.of("plains", "desert", "savanna", "snowy", "taiga");
    /** Router NICs: DOWN/UP faces are eth0/eth1 for any horizontal facing; the rest are logical. */
    public static final int ACCESS_PORT = 0, SERVER_PORT = 1, FIBER_PREV_PORT = 2, FIBER_NEXT_PORT = 3, UPLINK_PORT = 4;
    public static final int SERVER_NIC = 1;

    /** A ring site: AS 65000 + number, vanilla style, start height and fiber endpoint (above the patch panel). */
    public record Village(int number, int x, int z, String style, int groundY, int endY) {
        public BlockPos endpoint() {
            return new BlockPos(x, endY, z);
        }

        public BlockPos patchPanel() {
            return new BlockPos(x, endY - 1, z);
        }
    }

    public final List<Village> villages = new ArrayList<>();
    public final Set<String> cuts = new HashSet<>();
    /** Fiber path blocks that were removed (packed positions); chunks never generated count as intact. */
    public final Set<Long> brokenFiber = new HashSet<>();
    public final Map<String, Integer> cableBlocks = new HashMap<>();
    public final Map<String, long[]> nicPositions = new HashMap<>();
    public final Map<UUID, Integer> alwaysOn = new HashMap<>();
    private static final ExecutorService BOOT =
            Executors.newFixedThreadPool(
                    2,
                    r -> {
                        Thread t = new Thread(r, "ECM-Infrastructure-Boot");
                        t.setDaemon(true);
                        return t;
                    });
    /** Planned sites per world seed, read by world generation (placement, structure). */
    public static final Map<Long, List<Village>> SITES = new ConcurrentHashMap<>();
    /** Fiber ring per world seed, read by the fiber feature during world generation. */
    public static final Map<Long, FiberChords> FIBER = new ConcurrentHashMap<>();
    private volatile boolean stopping;
    private FiberChords ring;

    public static WorldNetwork get(ServerLevel level) {
        return level.getServer()
                .overworld()
                .getDataStorage()
                .computeIfAbsent(
                        new Factory<>(WorldNetwork::new, WorldNetwork::load), "ecm_network");
    }

    private static WorldNetwork load(CompoundTag tag, HolderLookup.Provider lookup) {
        WorldNetwork d = new WorldNetwork();
        ListTag list = tag.getList("villages", Tag.TAG_COMPOUND);
        for (int i = 0; i < list.size(); i++) {
            CompoundTag v = list.getCompound(i);
            // Saves from before the vanilla-village rebuild lack style/endpoint: re-plan those.
            if (!v.contains("style")) {
                d.villages.clear();
                break;
            }
            d.villages.add(new Village(v.getInt("number"), v.getInt("x"), v.getInt("z"), v.getString("style"),
                    v.getInt("groundY"), v.getInt("endY")));
        }
        for (Tag c : tag.getList("cuts", Tag.TAG_STRING)) d.cuts.add(c.getAsString());
        for (long p : tag.getLongArray("brokenFiber")) d.brokenFiber.add(p);
        CompoundTag blocks = tag.getCompound("cableBlocks");
        for (String key : blocks.getAllKeys()) d.cableBlocks.put(key, blocks.getInt(key));
        CompoundTag nics = tag.getCompound("nicPositions");
        for (String key : nics.getAllKeys()) d.nicPositions.put(key, nics.getLongArray(key));
        CompoundTag nodes = tag.getCompound("alwaysOn");
        for (String key : nodes.getAllKeys())
            try {
                d.alwaysOn.put(UUID.fromString(key), nodes.getInt(key));
            } catch (IllegalArgumentException ignored) {
            }
        return d;
    }

    public synchronized CompoundTag save(CompoundTag tag, HolderLookup.Provider lookup) {
        ListTag list = new ListTag();
        for (Village v : villages) {
            CompoundTag n = new CompoundTag();
            n.putInt("number", v.number);
            n.putInt("x", v.x);
            n.putInt("z", v.z);
            n.putString("style", v.style);
            n.putInt("groundY", v.groundY);
            n.putInt("endY", v.endY);
            list.add(n);
        }
        tag.put("villages", list);
        ListTag cut = new ListTag();
        for (String c : cuts) cut.add(StringTag.valueOf(c));
        tag.put("cuts", cut);
        tag.putLongArray("brokenFiber", brokenFiber.stream().mapToLong(Long::longValue).toArray());
        CompoundTag blocks = new CompoundTag();
        cableBlocks.forEach(blocks::putInt);
        tag.put("cableBlocks", blocks);
        CompoundTag nics = new CompoundTag();
        nicPositions.forEach(nics::putLongArray);
        tag.put("nicPositions", nics);
        CompoundTag nodes = new CompoundTag();
        alwaysOn.forEach((id, ports) -> nodes.putInt(id.toString(), ports));
        tag.put("alwaysOn", nodes);
        return tag;
    }

    private static final List<TagKey<Biome>> STYLE_TAGS = List.of(BiomeTags.HAS_VILLAGE_PLAINS,
            BiomeTags.HAS_VILLAGE_DESERT, BiomeTags.HAS_VILLAGE_SAVANNA, BiomeTags.HAS_VILLAGE_SNOWY, BiomeTags.HAS_VILLAGE_TAIGA);

    /** The vanilla village style for a biome; plains for biomes without villages. */
    public static String style(Holder<Biome> biome) {
        for (int i = 0; i < STYLE_TAGS.size(); i++) if (biome.is(STYLE_TAGS.get(i))) return STYLES.get(i);
        return "plains";
    }

    public static ResourceLocation ispTemplate(String style) {
        return ResourceLocation.fromNamespaceAndPath("evanscomputermod", "tech_village/isp_" + style);
    }

    /**
     * Height of the fiber endpoint above the start piece's ground: the patch panel's
     * template y plus one (the start piece's y = 0 layer sits at groundY - 1).
     */
    public static int endpointOffset(ServerLevel level, String style) {
        var scan = com.example.evanscomputermod.worldgen.TemplateScan.of(level.getStructureManager(), ispTemplate(style))
                .orElseThrow(() -> new IllegalStateException("Missing ISP template " + ispTemplate(style)));
        BlockPos panel = scan.find(com.example.evanscomputermod.block.ModBlocks.FIBER_PATCH_PANEL.get());
        if (panel == null) throw new IllegalStateException("ISP template has no patch panel: " + style);
        return panel.getY();
    }

    public synchronized void plan(ServerLevel level) {
        if (villages.isEmpty()) {
            var generator = level.getChunkSource().getGenerator();
            var random = level.getChunkSource().randomState();
            var sampler = random.sampler();
            BlockPos spawn = level.getSharedSpawnPos();
            double offset = RandomSource.create(level.getSeed()).nextDouble() * Math.PI * 2;
            for (int i = 1; i <= 10; i++) {
                double angle = offset + (i - 1) * Math.PI / 5;
                int x = spawn.getX() + (int) Math.round(5000 * Math.cos(angle));
                int z = spawn.getZ() + (int) Math.round(5000 * Math.sin(angle));
                // Steer the site to the nearest biome that has vanilla villages.
                var found =
                        generator
                                .getBiomeSource()
                                .findBiomeHorizontal(
                                        x,
                                        64,
                                        z,
                                        1536,
                                        16,
                                        b -> STYLE_TAGS.stream().anyMatch(b::is),
                                        RandomSource.create(level.getSeed() + i),
                                        true,
                                        sampler);
                if (found != null) {
                    x = found.getFirst().getX();
                    z = found.getFirst().getZ();
                }
                x = (x >> 4) << 4;
                z = (z >> 4) << 4;
                // Exactly JigsawStructure's start height: start_height 0 + first free WORLD_SURFACE_WG
                // height at the start piece's bounding-box centre, which is the mast column.
                int ground = generator.getFirstFreeHeight(x, z, Heightmap.Types.WORLD_SURFACE_WG, level, random);
                var biome = generator.getBiomeSource().getNoiseBiome(
                        QuartPos.fromBlock(x), QuartPos.fromBlock(ground), QuartPos.fromBlock(z), sampler);
                String style = style(biome);
                villages.add(new Village(i, x, z, style, ground, ground + endpointOffset(level, style)));
            }
            setDirty();
            EvansComputerMod.LOGGER.info("Tech Village ring planned: {}", villages);
        }
        SITES.put(level.getSeed(), List.copyOf(villages));
        if (ring == null) {
            ring = new FiberChords(villages.stream().map(v -> new int[] {v.x, v.endY, v.z}).toList());
            FIBER.put(level.getSeed(), ring);
        }
    }

    public FiberChords ring(ServerLevel level) {
        plan(level);
        return ring;
    }

    public static Village siteAt(long seed, int chunkX, int chunkZ) {
        for (Village v : SITES.getOrDefault(seed, List.of()))
            if ((v.x >> 4) == chunkX && (v.z >> 4) == chunkZ) return v;
        return null;
    }

    public UUID identity(ServerLevel level, int village, String role) {
        return UUID.nameUUIDFromBytes(
                (level.getSeed() + ":" + village + ":" + role).getBytes(StandardCharsets.UTF_8));
    }

    public Village nearest(BlockPos pos) {
        return villages.stream()
                .min(
                        Comparator.comparingDouble(
                                v -> Math.pow(v.x - pos.getX(), 2) + Math.pow(v.z - pos.getZ(), 2)))
                .orElseThrow();
    }

    public static String linkName(int a, int b) {
        return Math.min(a, b) + "-" + Math.max(a, b);
    }

    /** Chord index (0-based) joining village i and its next neighbour. */
    public static int chord(int a, int b) {
        int lo = Math.min(a, b), hi = Math.max(a, b);
        return lo == 1 && hi == 10 ? 9 : lo - 1;
    }

    public void link(ServerLevel level, int a, int b, boolean intact) {
        String name = linkName(a, b);
        if (intact) cuts.remove(name);
        else cuts.add(name);
        setDirty();
        applyLinks(level);
    }

    /** Physically intact (no removed path block) and not administratively cut. */
    public boolean linkUp(ServerLevel level, int a, int b) {
        return !cuts.contains(linkName(a, b)) && ring(level).intact(chord(a, b), brokenFiber);
    }

    /** A fiber block left the world: if it was on a chord path, that chord is cut. */
    public void fiberRemoved(ServerLevel level, BlockPos pos) {
        long p = FiberChords.pack(pos.getX(), pos.getY(), pos.getZ());
        if (ring(level).chordsAt(p) != 0 && brokenFiber.add(p)) {
            setDirty();
            applyLinks(level);
        }
    }

    /** A Fiber Span was placed: back on its path position, it repairs that break. */
    public void fiberPlaced(ServerLevel level, BlockPos pos) {
        if (brokenFiber.remove(FiberChords.pack(pos.getX(), pos.getY(), pos.getZ()))) {
            setDirty();
            applyLinks(level);
        }
    }

    private byte[] mac(ServerLevel level, int village, String role, int port) {
        return NetworkHub.deriveMac(identity(level, village, role), port);
    }

    public void applyLinks(ServerLevel level) {
        CableNetworkManager mgr = CableNetworkManager.getInstance();
        if (mgr == null) return;
        mgr.logicalLink("internet-isp1", mac(level, 1, "isp.router", UPLINK_PORT), InternetProxy.MAC, true);
        for (int i = 1; i <= 10; i++) {
            int next = i == 10 ? 1 : i + 1;
            mgr.logicalLink(
                    "fiber-" + linkName(i, next),
                    mac(level, i, "isp.router", FIBER_NEXT_PORT),
                    mac(level, next, "isp.router", FIBER_PREV_PORT),
                    linkUp(level, i, next));
            // Also cabled physically inside the ISP; the logical link keeps it up before terrain exists.
            mgr.logicalLink(
                    "server-" + i,
                    mac(level, i, "isp.router", SERVER_PORT),
                    mac(level, i, "isp.server", SERVER_NIC),
                    true);
        }
    }

    public static void start(ServerLevel level) {
        WorldNetwork d = get(level);
        d.plan(level);
        d.applyLinks(level);
        if (System.getProperty("neoforge.enabledGameTestNamespaces") != null) return;
        if (Boolean.parseBoolean(System.getProperty("evanscomputermod.techVillages", "true")))
            for (Village v : d.villages)
                for (String role : List.of("isp.router", "isp.server"))
                    d.boot(level, v.number, role);
        int count = 0;
        for (var node : Map.copyOf(d.alwaysOn).entrySet())
            if (count++ < Math.max(0, Integer.getInteger("evanscomputermod.headlessCap", 64)))
                d.bootInstance(level, node.getKey(), false, node.getValue());
    }

    public void boot(ServerLevel level, int number, String role) {
        UUID id = identity(level, number, role);
        ComputerHost host = ComputerHost.get(level.getServer(), id);
        host.setInfrastructure(true);
        if (host.instance() != null) return;
        provision(level, number, role);
        bootInstance(level, id, true, role.equals("isp.router") ? 9 : 5);
    }

    private void bootInstance(ServerLevel level, UUID id, boolean infrastructure, int ports) {
        ComputerHost host = ComputerHost.get(level.getServer(), id);
        host.setInfrastructure(infrastructure);
        if (!host.beginBoot()) return;
        BOOT.execute(
                () -> {
                    ComputerInstance instance = null;
                    try {
                        int safePorts = Math.max(1, Math.min(32, ports));
                        byte[][] macs = new byte[safePorts][];
                        for (int i = 0; i < safePorts; i++) macs[i] = NetworkHub.deriveMac(id, i);
                        instance = new ComputerInstance(host, macs);
                        instance.loadModule("terminal_os.wasm");
                        ComputerInstance ready = instance;
                        if (stopping) {
                            ready.close();
                            host.endBoot();
                            return;
                        }
                        level.getServer()
                                .execute(
                                        () -> {
                                            host.endBoot();
                                            if (stopping) {
                                                ready.close();
                                                return;
                                            }
                                            if (host.instance() != null) {
                                                ready.close();
                                                return;
                                            }
                                            host.track(ready);
                                            if (host.attachment()
                                                            instanceof
                                                            com.example.evanscomputermod.block
                                                                                    .TerminalBlockEntity
                                                                            block
                                                    && !block.isRemoved())
                                                block.attachLiveComputer(ready);
                                            try {
                                                ready.executeMain();
                                                ready.startWorkerThread();
                                            } catch (Exception e) {
                                                ready.close();
                                                host.forget();
                                                EvansComputerMod.LOGGER.error(
                                                        "Infrastructure boot failed", e);
                                            }
                                        });
                    } catch (Exception e) {
                        host.endBoot();
                        if (instance != null) instance.close();
                        EvansComputerMod.LOGGER.error("Infrastructure load failed", e);
                    }
                });
    }

    public void stop() {
        stopping = true;
    }

    public void provision(ServerLevel level, int number, String role) {
        Path root = ComputerStorage.path(level.getServer(), identity(level, number, role));
        try {
            Files.createDirectories(root);
            for (var e : configs(number, role).entrySet()) {
                Path p = root.resolve(e.getKey());
                if (!Files.exists(p)) Files.writeString(p, e.getValue());
            }
        } catch (Exception e) {
            throw new IllegalStateException("Cannot provision " + number + "/" + role, e);
        }
    }

    /**
     * Startup files. ISP router: eth0 (DOWN) village access LAN 100.(64+N).1.1/24 with a
     * DHCP pool for every house router and player PC on the village cable; eth1 (UP)
     * server LAN 100.(64+N).0.1/24; eth2/eth3 fiber to the previous/next village; eth4
     * village 1's uplink to the host gateway; eth5-eth8 spare (player AS peering).
     * House router: eth0 (DOWN) WAN by DHCP with NAT, eth1 (UP) private LAN; house PC and
     * server use their UP face eth1. DOWN/UP are eth0/eth1 for any horizontal facing.
     */
    public static Map<String, String> configs(int i, String role) {
        int previous = i == 1 ? 10 : i - 1, next = i == 10 ? 1 : i + 1, oct = 64 + i;
        Map<String, String> files = new HashMap<>();
        if (role.equals("isp.router")) {
            String cfg =
                    "configure terminal\nip routing\n"
                            + "interface eth0\nip address 100." + oct + ".1.1/24\nexit\n"
                            + "interface eth1\nip address 100." + oct + ".0.1/24\nexit\n"
                            + "interface eth2\nip address 172.31." + previous + ".2/30\nexit\n"
                            + "interface eth3\nip address 172.31." + i + ".1/30\nexit\n";
            if (i == 1)
                cfg +=
                        "interface eth0\nip nat inside\nexit\n"
                                + "interface eth1\nip nat inside\nexit\n"
                                + "interface eth2\nip nat inside\nexit\n"
                                + "interface eth3\nip nat inside\nexit\n"
                                + "interface eth4\nip address 10.0.0.2/24\nip nat outside\nexit\n"
                                + "ip route 0.0.0.0/0 10.0.0.1 eth4\n";
            cfg +=
                    "dhcp-server vrf default\npool village\nrange 100."
                            + oct + ".1.10 100." + oct + ".1.200\ndefault-router 100." + oct + ".1.1\n"
                            + "dns-server 1.1.1.1\nlease 86400\nenable\nexit\nexit\n";
            cfg +=
                    "router bgp " + (65000 + i)
                            + "\nbgp router-id 100." + oct + ".0.1\ntimers bgp 60 180\n"
                            + "neighbor 172.31." + previous + ".1 remote-as " + (65000 + previous) + "\n"
                            + "neighbor 172.31." + i + ".2 remote-as " + (65000 + next) + "\n"
                            + "address-family ipv4 unicast\n"
                            + "neighbor 172.31." + previous + ".1 activate\n"
                            + "neighbor 172.31." + i + ".2 activate\n"
                            + "network 100." + oct + ".0.0/24\n"
                            + "network 100." + oct + ".1.0/24\n";
            if (i == 1) cfg += "network 0.0.0.0/0\n";
            files.put("router.cfg", cfg + "end\n");
            files.put("services.cfg", "router on\nsshd &\n");
        } else if (role.equals("isp.server")) {
            files.put(
                    "network.cfg",
                    "iface eth1 100." + oct + ".0.10/24\nroute default via 100." + oct + ".0.1 dev eth1\n");
            files.put("services.cfg", "httpd 80 &\nsshd &\n");
            files.put("index.html", "<h1>Tech Village " + i + " — AS " + (65000 + i) + "</h1>\n");
        } else if (role.endsWith(".router")) {
            files.put("services.cfg", "router on\n");
            files.put(
                    "router.cfg",
                    "configure terminal\n"
                            + "ip routing\n"
                            + "interface eth0\n"
                            + "ip dhcp\n"
                            + "ip nat outside\n"
                            + "exit\n"
                            + "interface eth1\n"
                            + "ip address 192.168.1.1/24\n"
                            + "ip nat inside\n"
                            + "exit\n"
                            + "dhcp-server vrf default\n"
                            + "pool lan\n"
                            + "range 192.168.1.10 192.168.1.200\n"
                            + "default-router 192.168.1.1\n"
                            + "dns-server 1.1.1.1\n"
                            + "lease 86400\n"
                            + "enable\n"
                            + "end\n");
        } else {
            files.put("network.cfg", "iface eth1 dhcp\n");
        }
        return files;
    }
}
//?}
