package com.example.evanscomputermod.computer;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;

import net.minecraft.core.*;
import net.minecraft.nbt.*;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.tags.BiomeTags;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.saveddata.SavedData;

import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.*;

/** Durable village identities, logical fiber edges and last-known cable blocks. */
public final class WorldNetwork extends SavedData {
    public record Village(int number, int x, int z) {}

    public final List<Village> villages = new ArrayList<>();
    public final Set<String> cuts = new HashSet<>();
    public final Set<Long> brokenFiber = new HashSet<>();
    public final Set<Long> generatedFiber = new HashSet<>();
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
    public static final Map<Long, List<BlockPos>> SITES = new ConcurrentHashMap<>();
    private volatile boolean stopping;

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
            d.villages.add(new Village(v.getInt("number"), v.getInt("x"), v.getInt("z")));
        }
        for (Tag c : tag.getList("cuts", Tag.TAG_STRING)) d.cuts.add(c.getAsString());
        for (long p : tag.getLongArray("brokenFiber")) d.brokenFiber.add(p);
        for (long p : tag.getLongArray("generatedFiber")) d.generatedFiber.add(p);
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
            list.add(n);
        }
        tag.put("villages", list);
        ListTag cut = new ListTag();
        for (String c : cuts) cut.add(StringTag.valueOf(c));
        tag.put("cuts", cut);
        tag.putLongArray("brokenFiber", brokenFiber.stream().mapToLong(Long::longValue).toArray());
        tag.putLongArray(
                "generatedFiber", generatedFiber.stream().mapToLong(Long::longValue).toArray());
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

    public synchronized void plan(ServerLevel level) {
        if (villages.isEmpty()) {
            var generator = level.getChunkSource().getGenerator();
            var sampler = level.getChunkSource().randomState().sampler();
            BlockPos spawn = level.getSharedSpawnPos();
            double offset = RandomSource.create(level.getSeed()).nextDouble() * Math.PI * 2;
            for (int i = 1; i <= 10; i++) {
                double angle = offset + (i - 1) * Math.PI / 5;
                int x = spawn.getX() + (int) Math.round(5000 * Math.cos(angle));
                int z = spawn.getZ() + (int) Math.round(5000 * Math.sin(angle));
                var found =
                        generator
                                .getBiomeSource()
                                .findBiomeHorizontal(
                                        x,
                                        64,
                                        z,
                                        2048,
                                        4,
                                        b -> !b.is(BiomeTags.IS_OCEAN),
                                        RandomSource.create(level.getSeed() + i),
                                        true,
                                        sampler);
                if (found != null) {
                    x = found.getFirst().getX();
                    z = found.getFirst().getZ();
                }
                villages.add(new Village(i, (x >> 4) << 4, (z >> 4) << 4));
            }
            setDirty();
        }
        SITES.put(level.getSeed(), villages.stream().map(v -> new BlockPos(v.x, 64, v.z)).toList());
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

    public void link(ServerLevel level, int a, int b, boolean intact) {
        String name = linkName(a, b);
        if (intact) cuts.remove(name);
        else cuts.add(name);
        setDirty();
        applyLinks(level);
    }

    private byte[] mac(ServerLevel level, int village, String role, int port) {
        return NetworkHub.deriveMac(identity(level, village, role), port);
    }

    public void applyLinks(ServerLevel level) {
        CableNetworkManager mgr = CableNetworkManager.getInstance();
        if (mgr == null) return;
        mgr.logicalLink("internet-isp1", mac(level, 1, "isp.router", 4), InternetProxy.MAC, true);
        Set<String> broken = new HashSet<>();
        for (long p : brokenFiber) {
            String edge =
                    com.example.evanscomputermod.worldgen.FiberWorld.linkAt(this, BlockPos.of(p));
            if (edge != null) broken.add(edge);
        }
        for (int i = 1; i <= 10; i++) {
            int next = i == 10 ? 1 : i + 1;
            mgr.logicalLink(
                    "fiber-" + linkName(i, next),
                    mac(level, i, "isp.router", 1),
                    mac(level, next, "isp.router", 0),
                    !cuts.contains(linkName(i, next)) && !broken.contains(linkName(i, next)));
            mgr.logicalLink(
                    "server-" + i,
                    mac(level, i, "isp.router", 3),
                    mac(level, i, "isp.server", 0),
                    true);
            mgr.logicalLink(
                    "customer-" + i,
                    mac(level, i, "isp.router", 5),
                    mac(level, i, "isp.access", 8),
                    true);
            mgr.logicalLink(
                    "access-" + i,
                    mac(level, i, "isp.router", 2),
                    mac(level, i, "isp.access", 0),
                    true);
            for (int house = 1; house <= 6; house++) {
                mgr.logicalLink(
                        "house-wan-" + i + "-" + house,
                        mac(level, i, "isp.access", house),
                        mac(level, i, "house" + house + ".router", 1),
                        true);
                mgr.logicalLink(
                        "house-lan-" + i + "-" + house,
                        mac(level, i, "house" + house + ".router", 0),
                        mac(level, i, "house" + house + ".pc", 0),
                        true);
            }
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

    public static Map<String, String> configs(int i, String role) {
        int previous = i == 1 ? 10 : i - 1, next = i == 10 ? 1 : i + 1, oct = 64 + i;
        Map<String, String> files = new HashMap<>();
        if (role.equals("isp.router")) {
            String cfg =
                    "configure terminal\nip routing\ninterface eth0\nip address 172.31."
                            + previous
                            + ".2/30\nexit\ninterface eth1\nip address 172.31."
                            + i
                            + ".1/30\nexit\ninterface eth2\nip address 100."
                            + oct
                            + ".1.1/16\nexit\ninterface eth3\nip address 100."
                            + oct
                            + ".0.1/24\nexit\n";
            cfg += "interface eth5\nip address 100." + oct + ".2.1/24\nexit\n";
            if (i == 1)
                cfg +=
                        "interface eth5\n"
                            + "ip nat inside\n"
                            + "exit\n"
                            + "interface eth2\n"
                            + "ip nat inside\n"
                            + "exit\n"
                            + "interface eth0\n"
                            + "ip nat inside\n"
                            + "exit\n"
                            + "interface eth1\n"
                            + "ip nat inside\n"
                            + "exit\n"
                            + "interface eth3\n"
                            + "ip nat inside\n"
                            + "exit\n"
                            + "interface eth4\n"
                            + "ip address 10.0.0.2/24\n"
                            + "ip nat outside\n"
                            + "exit\n"
                            + "ip route 0.0.0.0/0 10.0.0.1 eth4\n";
            cfg +=
                    "dhcp-server vrf default\npool houses\nrange 100."
                            + oct
                            + ".1.10 100."
                            + oct
                            + ".1.200\ndefault-router 100."
                            + oct
                            + ".1.1\n"
                            + "dns-server 1.1.1.1\n"
                            + "lease 86400\n"
                            + "enable\n"
                            + "exit\n"
                            + "pool customers\n"
                            + "range 100."
                            + oct
                            + ".2.10 100."
                            + oct
                            + ".2.200\ndefault-router 100."
                            + oct
                            + ".2.1\ndns-server 1.1.1.1\nlease 86400\nenable\nexit\nexit\n";
            cfg +=
                    "router bgp "
                            + (65000 + i)
                            + "\nbgp router-id 100."
                            + oct
                            + ".0.1\ntimers bgp 60 180\nneighbor 172.31."
                            + previous
                            + ".1 remote-as "
                            + (65000 + previous)
                            + "\nneighbor 172.31."
                            + i
                            + ".2 remote-as "
                            + (65000 + next)
                            + "\naddress-family ipv4 unicast\nneighbor 172.31."
                            + previous
                            + ".1 activate\nneighbor 172.31."
                            + i
                            + ".2 activate\nnetwork 100."
                            + oct
                            + ".0.0/16\n";
            if (i == 1) cfg += "network 0.0.0.0/0\n";
            files.put("router.cfg", cfg + "end\n");
            files.put("services.cfg", "router on\nsshd &\n");
        } else if (role.equals("isp.server")) {
            files.put(
                    "network.cfg",
                    "iface eth0 100."
                            + oct
                            + ".0.10/24\nroute default via 100."
                            + oct
                            + ".0.1 dev eth0\n");
            files.put("services.cfg", "httpd 80 &\nsshd &\n");
            files.put("index.html", "<h1>Tech Village " + i + " — AS " + (65000 + i) + "</h1>\n");
        } else if (role.equals("isp.access")) {
            files.put("services.cfg", "switch on\n");
            files.put(
                    "switch.cfg",
                    "vlan 20\n"
                        + "name customers\n"
                        + "no shutdown\n"
                        + "exit\n"
                        + "interface eth0\n"
                        + "no routing\n"
                        + "exit\n"
                        + "interface eth1\n"
                        + "no routing\n"
                        + "exit\n"
                        + "interface eth2\n"
                        + "no routing\n"
                        + "exit\n"
                        + "interface eth3\n"
                        + "no routing\n"
                        + "exit\n"
                        + "interface eth4\n"
                        + "no routing\n"
                        + "exit\n"
                        + "interface eth5\n"
                        + "no routing\n"
                        + "exit\n"
                        + "interface eth6\n"
                        + "no routing\n"
                        + "exit\n"
                        + "interface eth7\n"
                        + "no routing\n"
                        + "vlan access 20\n"
                        + "exit\n"
                        + "interface eth8\n"
                        + "no routing\n"
                        + "vlan access 20\n"
                        + "exit\n");
        } else if (role.endsWith(".router")) {
            files.put("services.cfg", "router on\n");
            files.put(
                    "router.cfg",
                    "configure terminal\n"
                        + "ip routing\n"
                        + "interface eth0\n"
                        + "ip address 192.168.1.1/24\n"
                        + "ip nat inside\n"
                        + "exit\n"
                        + "interface eth1\n"
                        + "ip dhcp\n"
                        + "ip nat outside\n"
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
            files.put("network.cfg", "iface eth0 dhcp\n");
        }
        return files;
    }
}
//?}
